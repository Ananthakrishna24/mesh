use std::sync::{Arc, Mutex};

use mesh_core::NodeId;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{
    ClientConfig, DigitallySignedStruct, DistinguishedName, Error as TlsError, ServerConfig,
    SignatureScheme,
};

use crate::{NetError, NetResult};

#[derive(Debug)]
pub struct ObservedCertificate {
    inner: Mutex<Option<Vec<u8>>>,
}

impl ObservedCertificate {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(None),
        })
    }

    pub fn set(&self, certificate_der: &[u8]) {
        *self.inner.lock().expect("certificate lock") = Some(certificate_der.to_vec());
    }

    pub fn get(&self) -> Option<Vec<u8>> {
        self.inner.lock().expect("certificate lock").clone()
    }

}

#[derive(Debug)]
struct MeshServerCertVerifier {
    expected: Option<NodeId>,
    observed: Arc<ObservedCertificate>,
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl MeshServerCertVerifier {
    fn new(expected: Option<NodeId>, observed: Arc<ObservedCertificate>) -> Arc<Self> {
        Arc::new(Self {
            expected,
            observed,
            provider: Arc::new(rustls::crypto::ring::default_provider()),
        })
    }
}

impl ServerCertVerifier for MeshServerCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        let node_id = NodeId::from_certificate_der(end_entity.as_ref());
        if let Some(expected) = self.expected {
            if node_id != expected {
                return Err(TlsError::General(
                    "server certificate node id mismatch".to_owned(),
                ));
            }
        }
        self.observed.set(end_entity.as_ref());
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[derive(Debug)]
struct MeshClientCertVerifier {
    allow_unknown: bool,
    allowed: Mutex<Vec<NodeId>>,
    observed: Arc<ObservedCertificate>,
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl MeshClientCertVerifier {
    fn new(allow_unknown: bool, allowed: Vec<NodeId>, observed: Arc<ObservedCertificate>) -> Arc<Self> {
        Arc::new(Self {
            allow_unknown,
            allowed: Mutex::new(allowed),
            observed,
            provider: Arc::new(rustls::crypto::ring::default_provider()),
        })
    }
}

impl ClientCertVerifier for MeshClientCertVerifier {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, TlsError> {
        let node_id = NodeId::from_certificate_der(end_entity.as_ref());
        let allowed = self.allowed.lock().expect("allowed lock");
        if self.allow_unknown || allowed.iter().any(|allowed| *allowed == node_id) {
            self.observed.set(end_entity.as_ref());
            Ok(ClientCertVerified::assertion())
        } else {
            Err(TlsError::General(
                "client certificate is not in peer store".to_owned(),
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

pub fn build_server_config(
    cert: CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
    allow_unknown_clients: bool,
    allowed_clients: Vec<NodeId>,
    observed: Arc<ObservedCertificate>,
) -> NetResult<quinn::ServerConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut server = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| NetError::Tls(error.to_string()))?
        .with_client_cert_verifier(MeshClientCertVerifier::new(
            allow_unknown_clients,
            allowed_clients,
            observed,
        ))
        .with_single_cert(vec![cert], key)
        .map_err(|error| NetError::Tls(error.to_string()))?;
    server.alpn_protocols = vec![b"mesh/1".to_vec()];

    let mut transport = quinn::TransportConfig::default();
    transport.keep_alive_interval(Some(std::time::Duration::from_secs(15)));
    transport.max_idle_timeout(Some(
        std::time::Duration::from_secs(60)
            .try_into()
            .expect("idle timeout"),
    ));

    let mut server_config = quinn::ServerConfig::with_crypto(Arc::new(
        quinn::crypto::rustls::QuicServerConfig::try_from(server)
            .map_err(|error| NetError::Tls(error.to_string()))?,
    ));
    server_config.transport_config(Arc::new(transport));
    Ok(server_config)
}

pub fn build_client_config(
    cert: CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
    expected_server: Option<NodeId>,
    observed: Arc<ObservedCertificate>,
) -> NetResult<quinn::ClientConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut client = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| NetError::Tls(error.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(MeshServerCertVerifier::new(expected_server, observed))
        .with_client_auth_cert(vec![cert], key)
        .map_err(|error| NetError::Tls(error.to_string()))?;
    client.alpn_protocols = vec![b"mesh/1".to_vec()];

    let mut transport = quinn::TransportConfig::default();
    transport.keep_alive_interval(Some(std::time::Duration::from_secs(15)));
    transport.max_idle_timeout(Some(
        std::time::Duration::from_secs(60)
            .try_into()
            .expect("idle timeout"),
    ));

    let mut client_config = quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(client)
            .map_err(|error| NetError::Tls(error.to_string()))?,
    ));
    client_config.transport_config(Arc::new(transport));
    Ok(client_config)
}
