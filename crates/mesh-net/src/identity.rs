use mesh_core::{LocalIdentity, NodeId};
use rcgen::{CertificateParams, DistinguishedName, DnType, IsCa, KeyPair, KeyUsagePurpose};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

use crate::{NetError, NetResult};

pub struct GeneratedCertificate {
    pub certificate_der: Vec<u8>,
    pub private_key_der: Vec<u8>,
    pub node_id: NodeId,
}

pub fn generate_node_certificate() -> NetResult<GeneratedCertificate> {
    let key_pair = KeyPair::generate().map_err(|error| NetError::Identity(error.to_string()))?;
    let mut params = CertificateParams::new(vec!["mesh-node".to_owned()])
        .map_err(|error| NetError::Identity(error.to_string()))?;
    params.is_ca = IsCa::NoCa;
    params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyEncipherment,
    ];
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "mesh-node");
    params.distinguished_name = dn;

    let certificate = params
        .self_signed(&key_pair)
        .map_err(|error| NetError::Identity(error.to_string()))?;
    let certificate_der = certificate.der().as_ref().to_vec();
    let private_key_der = key_pair.serialize_der();
    let node_id = NodeId::from_certificate_der(&certificate_der);

    Ok(GeneratedCertificate {
        certificate_der,
        private_key_der,
        node_id,
    })
}

pub fn load_identity_material(
    identity: &LocalIdentity,
) -> NetResult<(CertificateDer<'static>, PrivateKeyDer<'static>)> {
    let cert = CertificateDer::from(identity.certificate_der.clone());
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(identity.private_key_der.clone()));
    Ok((cert, key))
}
