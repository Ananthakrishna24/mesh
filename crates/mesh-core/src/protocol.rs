use bytes::Bytes;
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

pub const PROTOCOL_MAJOR: u32 = 1;
pub const PROTOCOL_MINOR: u32 = 0;
pub const PROTOCOL_MINOR_MIN: u32 = 0;
pub const MAX_CONTROL_FRAME_BYTES: usize = 1_048_576;
pub const MESSAGE_ID_LEN: usize = 16;

pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/mesh.v1.rs"));
}

pub fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

pub fn random_message_id() -> Bytes {
    let mut bytes = [0u8; MESSAGE_ID_LEN];
    rand::rng().fill_bytes(&mut bytes);
    Bytes::copy_from_slice(&bytes)
}

pub fn capability_digest(parts: &[&[u8]]) -> Bytes {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    Bytes::copy_from_slice(&hasher.finalize())
}
