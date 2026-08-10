use std::net::{Ipv6Addr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::Duration;

use mesh_core::{LocalIdentity, NodeId};
use quinn::{Connection, Endpoint, EndpointConfig, default_runtime};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::time::timeout;
use tracing::info;

use crate::identity::load_identity_material;
use crate::tls::{ObservedCertificate, build_client_config, build_server_config};
use crate::{NetError, NetResult};

#[derive(Debug, Clone)]
pub struct PeerConnection {
    pub connection: Connection,
    pub peer_node_id: NodeId,
    pub peer_certificate_der: Vec<u8>,
    pub remote_address: SocketAddr,
}

#[derive(Debug, Clone)]
pub struct IncomingPeer {
    pub connection: Connection,
    pub peer_node_id: NodeId,
    pub peer_certificate_der: Vec<u8>,
    pub remote_address: SocketAddr,
}

#[derive(Clone)]
pub struct MeshEndpoint {
    endpoint: Endpoint,
    identity: LocalIdentity,
    listen_addr: SocketAddr,
    client_observed: Arc<ObservedCertificate>,
    cert: CertificateDer<'static>,
    key_bytes: Vec<u8>,
}

impl MeshEndpoint {
    pub fn bind(identity: LocalIdentity, bind_addr: SocketAddr) -> NetResult<Self> {
        let socket = UdpSocket::bind(bind_addr)?;
        Self::from_udp_socket(identity, socket)
    }

    pub fn bind_wildcard(identity: LocalIdentity, port: u16) -> NetResult<Self> {
        let socket = bind_wildcard_udp(port)?;
        Self::from_udp_socket(identity, socket)
    }

    pub fn from_udp_socket(identity: LocalIdentity, socket: UdpSocket) -> NetResult<Self> {
        let (cert, key) = load_identity_material(&identity)?;
        let key_bytes = private_key_bytes(&key);
        let server_observed = ObservedCertificate::new();
        let client_observed = ObservedCertificate::new();
        let server_config =
            build_server_config(cert.clone(), key, true, Vec::new(), server_observed)?;
        let runtime = default_runtime().ok_or_else(|| {
            NetError::Io(std::io::Error::other("no async runtime found for quinn"))
        })?;
        let endpoint = Endpoint::new(
            EndpointConfig::default(),
            Some(server_config),
            socket,
            runtime,
        )?;
        let listen_addr = endpoint.local_addr()?;
        info!(%listen_addr, "quic endpoint bound");

        let mut mesh = Self {
            endpoint,
            identity,
            listen_addr,
            client_observed,
            cert,
            key_bytes,
        };
        mesh.refresh_client_config(None)?;
        Ok(mesh)
    }

    pub fn listen_addr(&self) -> SocketAddr {
        self.listen_addr
    }

    pub fn identity(&self) -> &LocalIdentity {
        &self.identity
    }

    pub fn refresh_client_config(&mut self, expected_server: Option<NodeId>) -> NetResult<()> {
        let client_config = build_client_config(
            self.cert.clone(),
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.key_bytes.clone())),
            expected_server,
            self.client_observed.clone(),
        )?;
        self.endpoint.set_default_client_config(client_config);
        Ok(())
    }

    pub async fn accept(&self) -> NetResult<IncomingPeer> {
        let incoming = self.endpoint.accept().await.ok_or(NetError::Closed)?;
        let connection = incoming.await?;
        let certificate = peer_certificate(&connection).ok_or_else(|| {
            NetError::Identity("accepted connection without peer certificate".to_owned())
        })?;
        let peer_node_id = NodeId::from_certificate_der(&certificate);
        Ok(IncomingPeer {
            remote_address: connection.remote_address(),
            connection,
            peer_node_id,
            peer_certificate_der: certificate,
        })
    }

    pub async fn connect(
        &mut self,
        addr: SocketAddr,
        expected_server: NodeId,
    ) -> NetResult<PeerConnection> {
        self.refresh_client_config(Some(expected_server))?;
        let connecting = self.endpoint.connect(addr, "mesh-node")?;
        let connection = timeout(Duration::from_secs(10), connecting)
            .await
            .map_err(|_| NetError::Timeout)??;
        let certificate = peer_certificate(&connection)
            .or_else(|| self.client_observed.get())
            .ok_or_else(|| NetError::Identity("connected without server certificate".to_owned()))?;
        let peer_node_id = NodeId::from_certificate_der(&certificate);
        if peer_node_id != expected_server {
            return Err(NetError::Identity(
                "connected server node id mismatch".to_owned(),
            ));
        }
        Ok(PeerConnection {
            remote_address: connection.remote_address(),
            connection,
            peer_node_id,
            peer_certificate_der: certificate,
        })
    }

    pub fn close(&self) {
        self.endpoint.close(0u32.into(), b"shutdown");
    }
}

fn bind_wildcard_udp(port: u16) -> std::io::Result<UdpSocket> {
    let address = SocketAddr::from((Ipv6Addr::UNSPECIFIED, port));
    let dual_stack =
        Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP)).and_then(|socket| {
            socket.set_only_v6(false)?;
            socket.bind(&address.into())?;
            Ok(socket)
        });
    match dual_stack {
        Ok(socket) => Ok(socket.into()),
        Err(error) => {
            tracing::debug!(%error, "dual-stack UDP unavailable; falling back to IPv4");
            UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], port)))
        }
    }
}

fn peer_certificate(connection: &Connection) -> Option<Vec<u8>> {
    let identity = connection.peer_identity()?;
    if let Some(certs) = identity.downcast_ref::<Vec<CertificateDer<'static>>>() {
        return certs.first().map(|cert| cert.as_ref().to_vec());
    }
    if let Some(certs) = identity.downcast_ref::<Vec<CertificateDer<'_>>>() {
        return certs.first().map(|cert| cert.as_ref().to_vec());
    }
    None
}

fn private_key_bytes(key: &PrivateKeyDer<'static>) -> Vec<u8> {
    match key {
        PrivateKeyDer::Pkcs8(key) => key.secret_pkcs8_der().to_vec(),
        PrivateKeyDer::Sec1(key) => key.secret_sec1_der().to_vec(),
        PrivateKeyDer::Pkcs1(key) => key.secret_pkcs1_der().to_vec(),
        _ => panic!("unsupported private key type"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesh_core::{MeshId, now_unix_ms};

    #[test]
    fn wildcard_socket_uses_dual_stack_when_available() {
        let socket = bind_wildcard_udp(0).expect("bind wildcard UDP socket");
        if socket.local_addr().expect("local address").is_ipv4() {
            return;
        }
        let socket = Socket::from(socket);
        assert!(!socket.only_v6().expect("read IPV6_V6ONLY"));
    }

    #[tokio::test]
    async fn wildcard_endpoint_connects_to_ipv6() {
        let mesh_id = MeshId::new();
        let server_identity = test_identity("Server", mesh_id);
        let client_identity = test_identity("Client", mesh_id);
        let Ok(server) = MeshEndpoint::bind(
            server_identity.clone(),
            SocketAddr::from((Ipv6Addr::LOCALHOST, 0)),
        ) else {
            return;
        };
        let server_address = server.listen_addr();
        let accept = tokio::spawn(async move { server.accept().await });

        let mut client =
            MeshEndpoint::bind_wildcard(client_identity, 0).expect("bind wildcard endpoint");
        if client.listen_addr().is_ipv4() {
            return;
        }
        let connected = client
            .connect(server_address, server_identity.node_id)
            .await
            .expect("connect over IPv6");
        let incoming = accept.await.expect("accept task").expect("accept peer");

        assert_eq!(connected.remote_address, server_address);
        assert_eq!(incoming.peer_node_id, client.identity().node_id);
    }

    fn test_identity(display_name: &str, mesh_id: MeshId) -> LocalIdentity {
        let generated = crate::generate_node_certificate().expect("generate certificate");
        LocalIdentity {
            node_id: generated.node_id,
            mesh_id,
            display_name: display_name.to_owned(),
            certificate_der: generated.certificate_der,
            private_key_der: generated.private_key_der,
            created_at_unix_ms: now_unix_ms(),
        }
    }
}
