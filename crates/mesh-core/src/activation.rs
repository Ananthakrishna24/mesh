use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{DeploymentId, RequestId};

pub const ACTIVATION_HEADER_BYTES: usize = 128;
pub const ACTIVATION_MAGIC: &[u8; 4] = b"MSHA";
pub const ACTIVATION_WIRE_MAJOR: u16 = 1;
pub const ACTIVATION_WIRE_MINOR: u16 = 0;
pub const ACTIVATION_MAX_RANK: u8 = 4;
pub const ACTIVATION_MAX_PAYLOAD_BYTES: u64 = 268_435_456;
pub const ACTIVATION_MAX_IN_FLIGHT_PER_STAGE_REQUEST: u32 = 2;
pub const ACTIVATION_DTYPE_FP16: u8 = 1;
pub const ACTIVATION_BYTES_PER_FP16: u64 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum TransferKind {
    Prefill = 1,
    Decode = 2,
}

impl TransferKind {
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Prefill),
            2 => Some(Self::Decode),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prefill => "prefill",
            Self::Decode => "decode",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ActivationValidationError {
    #[error("unsupported activation protocol major {0}")]
    UnsupportedProtocol(u16),
    #[error("malformed activation frame: {0}")]
    MalformedFrame(&'static str),
    #[error("transfer rejected: {0}")]
    TransferRejected(&'static str),
    #[error("invalid activation state: {0}")]
    InvalidState(&'static str),
    #[error("activation resource busy: {0}")]
    ResourceBusy(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivationHeader {
    pub wire_major: u16,
    pub wire_minor: u16,
    pub deployment_id: DeploymentId,
    pub request_id: RequestId,
    pub transfer_id: u64,
    pub source_stage: u16,
    pub destination_stage: u16,
    pub transfer_kind: TransferKind,
    pub data_type: u8,
    pub rank: u8,
    pub flags: u8,
    pub dimensions: [u64; 4],
    pub sequence_position: u64,
    pub payload_len: u64,
    pub element_count: u64,
}

impl ActivationHeader {
    pub fn qwen3_hidden(
        deployment_id: DeploymentId,
        request_id: RequestId,
        transfer_id: u64,
        source_stage: u16,
        destination_stage: u16,
        transfer_kind: TransferKind,
        batch: u64,
        sequence: u64,
        hidden: u64,
        sequence_position: u64,
    ) -> Result<Self, ActivationValidationError> {
        let rank = 3;
        let dimensions = [batch, sequence, hidden, 0];
        let element_count = checked_product(&dimensions[..rank as usize])?;
        let payload_len = element_count
            .checked_mul(ACTIVATION_BYTES_PER_FP16)
            .ok_or(ActivationValidationError::MalformedFrame(
                "payload length overflow",
            ))?;
        let header = Self {
            wire_major: ACTIVATION_WIRE_MAJOR,
            wire_minor: ACTIVATION_WIRE_MINOR,
            deployment_id,
            request_id,
            transfer_id,
            source_stage,
            destination_stage,
            transfer_kind,
            data_type: ACTIVATION_DTYPE_FP16,
            rank,
            flags: 0,
            dimensions,
            sequence_position,
            payload_len,
            element_count,
        };
        header.validate_shape_only()?;
        Ok(header)
    }

    pub fn used_dimensions(&self) -> &[u64] {
        &self.dimensions[..self.rank as usize]
    }

    pub fn validate_shape_only(&self) -> Result<(), ActivationValidationError> {
        if self.wire_major != ACTIVATION_WIRE_MAJOR {
            return Err(ActivationValidationError::UnsupportedProtocol(
                self.wire_major,
            ));
        }
        if self.flags != 0 {
            return Err(ActivationValidationError::MalformedFrame(
                "flags must be zero",
            ));
        }
        if self.rank == 0 || self.rank > ACTIVATION_MAX_RANK {
            return Err(ActivationValidationError::MalformedFrame("rank out of range"));
        }
        if self.data_type != ACTIVATION_DTYPE_FP16 {
            return Err(ActivationValidationError::TransferRejected(
                "unknown activation data type",
            ));
        }
        if TransferKind::from_u8(self.transfer_kind.as_u8()).is_none() {
            return Err(ActivationValidationError::TransferRejected(
                "unknown transfer kind",
            ));
        }
        if self.destination_stage != self.source_stage.saturating_add(1) {
            return Err(ActivationValidationError::MalformedFrame(
                "destination stage must be source+1",
            ));
        }
        for (index, dim) in self.dimensions.iter().enumerate() {
            if index < self.rank as usize {
                if *dim == 0 {
                    return Err(ActivationValidationError::MalformedFrame(
                        "zero-sized dimension",
                    ));
                }
            } else if *dim != 0 {
                return Err(ActivationValidationError::MalformedFrame(
                    "unused dimension must be zero",
                ));
            }
        }
        let expected_elements = checked_product(self.used_dimensions())?;
        if expected_elements != self.element_count {
            return Err(ActivationValidationError::MalformedFrame(
                "element_count mismatch",
            ));
        }
        let expected_payload = expected_elements
            .checked_mul(ACTIVATION_BYTES_PER_FP16)
            .ok_or(ActivationValidationError::MalformedFrame(
                "payload length overflow",
            ))?;
        if expected_payload != self.payload_len {
            return Err(ActivationValidationError::MalformedFrame(
                "payload_len mismatch",
            ));
        }
        if self.payload_len > ACTIVATION_MAX_PAYLOAD_BYTES {
            return Err(ActivationValidationError::MalformedFrame(
                "payload exceeds protocol maximum",
            ));
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<[u8; ACTIVATION_HEADER_BYTES], ActivationValidationError> {
        self.validate_shape_only()?;
        let mut out = [0u8; ACTIVATION_HEADER_BYTES];
        out[0..4].copy_from_slice(ACTIVATION_MAGIC);
        out[4..6].copy_from_slice(&self.wire_major.to_be_bytes());
        out[6..8].copy_from_slice(&self.wire_minor.to_be_bytes());
        out[8..24].copy_from_slice(self.deployment_id.as_bytes());
        out[24..40].copy_from_slice(self.request_id.as_bytes());
        out[40..48].copy_from_slice(&self.transfer_id.to_be_bytes());
        out[48..50].copy_from_slice(&self.source_stage.to_be_bytes());
        out[50..52].copy_from_slice(&self.destination_stage.to_be_bytes());
        out[52] = self.transfer_kind.as_u8();
        out[53] = self.data_type;
        out[54] = self.rank;
        out[55] = self.flags;
        for (index, dim) in self.dimensions.iter().enumerate() {
            let start = 56 + index * 8;
            out[start..start + 8].copy_from_slice(&dim.to_be_bytes());
        }
        out[88..96].copy_from_slice(&self.sequence_position.to_be_bytes());
        out[96..104].copy_from_slice(&self.payload_len.to_be_bytes());
        out[104..112].copy_from_slice(&self.element_count.to_be_bytes());
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ActivationValidationError> {
        if bytes.len() != ACTIVATION_HEADER_BYTES {
            return Err(ActivationValidationError::MalformedFrame(
                "header must be 128 bytes",
            ));
        }
        if &bytes[0..4] != ACTIVATION_MAGIC.as_slice() {
            return Err(ActivationValidationError::MalformedFrame("bad magic"));
        }
        let wire_major = u16::from_be_bytes([bytes[4], bytes[5]]);
        if wire_major != ACTIVATION_WIRE_MAJOR {
            return Err(ActivationValidationError::UnsupportedProtocol(wire_major));
        }
        let wire_minor = u16::from_be_bytes([bytes[6], bytes[7]]);
        let deployment_id = DeploymentId::from_slice(&bytes[8..24]).map_err(|_| {
            ActivationValidationError::MalformedFrame("invalid deployment id")
        })?;
        let request_id = RequestId::from_slice(&bytes[24..40])
            .map_err(|_| ActivationValidationError::MalformedFrame("invalid request id"))?;
        let transfer_id = u64::from_be_bytes(bytes[40..48].try_into().unwrap());
        let source_stage = u16::from_be_bytes([bytes[48], bytes[49]]);
        let destination_stage = u16::from_be_bytes([bytes[50], bytes[51]]);
        let transfer_kind = TransferKind::from_u8(bytes[52]).ok_or(
            ActivationValidationError::TransferRejected("unknown transfer kind"),
        )?;
        let data_type = bytes[53];
        let rank = bytes[54];
        let flags = bytes[55];
        let mut dimensions = [0u64; 4];
        for index in 0..4 {
            let start = 56 + index * 8;
            dimensions[index] =
                u64::from_be_bytes(bytes[start..start + 8].try_into().unwrap());
        }
        let sequence_position = u64::from_be_bytes(bytes[88..96].try_into().unwrap());
        let payload_len = u64::from_be_bytes(bytes[96..104].try_into().unwrap());
        let element_count = u64::from_be_bytes(bytes[104..112].try_into().unwrap());
        if bytes[112..128].iter().any(|byte| *byte != 0) {
            return Err(ActivationValidationError::MalformedFrame(
                "reserved bytes must be zero",
            ));
        }
        let header = Self {
            wire_major,
            wire_minor,
            deployment_id,
            request_id,
            transfer_id,
            source_stage,
            destination_stage,
            transfer_kind,
            data_type,
            rank,
            flags,
            dimensions,
            sequence_position,
            payload_len,
            element_count,
        };
        header.validate_shape_only()?;
        Ok(header)
    }
}

fn checked_product(dims: &[u64]) -> Result<u64, ActivationValidationError> {
    let mut product = 1u64;
    for dim in dims {
        product = product
            .checked_mul(*dim)
            .ok_or(ActivationValidationError::MalformedFrame(
                "element count overflow",
            ))?;
    }
    Ok(product)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_qwen3_header() {
        let header = ActivationHeader::qwen3_hidden(
            DeploymentId::from_bytes([1; 16]),
            RequestId::from_bytes([2; 16]),
            7,
            0,
            1,
            TransferKind::Prefill,
            1,
            4,
            2560,
            0,
        )
        .unwrap();
        let bytes = header.encode().unwrap();
        assert_eq!(bytes.len(), 128);
        let decoded = ActivationHeader::decode(&bytes).unwrap();
        assert_eq!(decoded, header);
        assert_eq!(decoded.payload_len, 1 * 4 * 2560 * 2);
    }

    #[test]
    fn rejects_bad_magic_and_zero_dim() {
        let mut bytes = ActivationHeader::qwen3_hidden(
            DeploymentId::from_bytes([1; 16]),
            RequestId::from_bytes([2; 16]),
            1,
            0,
            1,
            TransferKind::Decode,
            1,
            1,
            2560,
            3,
        )
        .unwrap()
        .encode()
        .unwrap();
        bytes[0] = b'X';
        assert!(matches!(
            ActivationHeader::decode(&bytes),
            Err(ActivationValidationError::MalformedFrame("bad magic"))
        ));

        let err = ActivationHeader::qwen3_hidden(
            DeploymentId::from_bytes([1; 16]),
            RequestId::from_bytes([2; 16]),
            1,
            0,
            1,
            TransferKind::Decode,
            1,
            0,
            2560,
            0,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ActivationValidationError::MalformedFrame("zero-sized dimension")
        ));
    }
}
