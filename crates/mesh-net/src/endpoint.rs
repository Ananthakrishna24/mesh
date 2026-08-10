use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use mesh_core::{LocalIdentity, NodeId};
use quinn::{Connection, Endpoint};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
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
        let (cert, key) = load_identity_material(&identity)?;
        let key_bytes = private_key_bytes(&key);
        let server_observed = ObservedCertificate::new();
        let client_observed = ObservedCertificate::new();
        let server_config = build_server_config(
            cert.clone(),
            key,
            true,
            Vec::new(),
            server_observed,
        )?;
        let endpoint = Endpoint::server(server_config, bind_addr)?;
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
            .ok_or_else(|| {
                NetError::Identity("connected without server certificate".to_owned())
            })?;
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
