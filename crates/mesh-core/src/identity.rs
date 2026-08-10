use serde::{Deserialize, Serialize};

use crate::{MeshId, NodeId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalIdentity {
    pub node_id: NodeId,
    pub mesh_id: MeshId,
    pub display_name: String,
    pub certificate_der: Vec<u8>,
    pub private_key_der: Vec<u8>,
    pub created_at_unix_ms: i64,
}

pub fn identity_matches(identity: &LocalIdentity) -> bool {
    NodeId::from_certificate_der(&identity.certificate_der) == identity.node_id
        && !identity.private_key_der.is_empty()
        && !identity.display_name.trim().is_empty()
}
