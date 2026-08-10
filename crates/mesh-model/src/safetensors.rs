use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

use mesh_core::{MAX_SAFETENSORS_HEADER_BYTES, RANGE_MERGE_GAP_BYTES};

use crate::{ModelError, ModelResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafetensorsDtype {
    Bool,
    U8,
    I8,
    I16,
    U16,
    F16,
    Bf16,
    I32,
    U32,
    F32,
    I64,
    U64,
    F64,
}

impl SafetensorsDtype {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "BOOL" => Some(Self::Bool),
            "U8" => Some(Self::U8),
            "I8" => Some(Self::I8),
            "I16" => Some(Self::I16),
            "U16" => Some(Self::U16),
            "F16" => Some(Self::F16),
            "BF16" => Some(Self::Bf16),
            "I32" => Some(Self::I32),
            "U32" => Some(Self::U32),
            "F32" => Some(Self::F32),
            "I64" => Some(Self::I64),
            "U64" => Some(Self::U64),
            "F64" => Some(Self::F64),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bool => "BOOL",
            Self::U8 => "U8",
            Self::I8 => "I8",
            Self::I16 => "I16",
            Self::U16 => "U16",
            Self::F16 => "F16",
            Self::Bf16 => "BF16",
            Self::I32 => "I32",
            Self::U32 => "U32",
            Self::F32 => "F32",
            Self::I64 => "I64",
            Self::U64 => "U64",
            Self::F64 => "F64",
        }
    }
}

pub fn dtype_width_bytes(dtype: SafetensorsDtype) -> u64 {
    match dtype {
        SafetensorsDtype::Bool | SafetensorsDtype::U8 | SafetensorsDtype::I8 => 1,
        SafetensorsDtype::I16
        | SafetensorsDtype::U16
        | SafetensorsDtype::F16
        | SafetensorsDtype::Bf16 => 2,
        SafetensorsDtype::I32 | SafetensorsDtype::U32 | SafetensorsDtype::F32 => 4,
        SafetensorsDtype::I64 | SafetensorsDtype::U64 | SafetensorsDtype::F64 => 8,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafetensorsTensorInfo {
    pub dtype: SafetensorsDtype,
    pub shape: Vec<u64>,
    pub data_offsets: (u64, u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafetensorsHeader {
    pub header_length: u64,
    pub tensors: BTreeMap<String, SafetensorsTensorInfo>,
    pub metadata: BTreeMap<String, String>,
}

pub fn parse_header_length(prefix: &[u8]) -> ModelResult<u64> {
    if prefix.len() < 8 {
        return Err(ModelError::Invalid(
            "safetensors header length requires 8 bytes".to_owned(),
        ));
    }
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&prefix[..8]);
    let header_length = u64::from_le_bytes(bytes);
    if header_length == 0 || header_length > MAX_SAFETENSORS_HEADER_BYTES {
        return Err(ModelError::Invalid(format!(
            "safetensors header length {header_length} out of bounds"
        )));
    }
    Ok(header_length)
}

pub fn parse_safetensors_header(header_length: u64, header_json: &[u8]) -> ModelResult<SafetensorsHeader> {
    if header_json.len() as u64 != header_length {
        return Err(ModelError::Invalid(format!(
            "safetensors header bytes {} != declared length {header_length}",
            header_json.len()
        )));
    }

    let root: BTreeMap<String, Value> = serde_json::from_slice(header_json)?;
    let mut tensors = BTreeMap::new();
    let mut metadata = BTreeMap::new();

    for (name, value) in root {
        if name == "__metadata__" {
            if let Some(object) = value.as_object() {
                for (key, meta_value) in object {
                    if let Some(text) = meta_value.as_str() {
                        metadata.insert(key.clone(), text.to_owned());
                    } else {
                        metadata.insert(key.clone(), meta_value.to_string());
                    }
                }
            }
            continue;
        }

        let info: RawTensorInfo = serde_json::from_value(value).map_err(|error| {
            ModelError::Invalid(format!("tensor {name} header invalid: {error}"))
        })?;
        let dtype = SafetensorsDtype::parse(&info.dtype).ok_or_else(|| {
            ModelError::Unsupported(format!("tensor {name} has unsupported dtype {}", info.dtype))
        })?;
        if info.data_offsets.len() != 2 {
            return Err(ModelError::Invalid(format!(
                "tensor {name} data_offsets must have two entries"
            )));
        }
        let start = info.data_offsets[0];
        let end = info.data_offsets[1];
        if end < start {
            return Err(ModelError::Invalid(format!(
                "tensor {name} data_offsets end before start"
            )));
        }
        tensors.insert(
            name,
            SafetensorsTensorInfo {
                dtype,
                shape: info.shape,
                data_offsets: (start, end),
            },
        );
    }

    Ok(SafetensorsHeader {
        header_length,
        tensors,
        metadata,
    })
}

pub fn tensor_payload_absolute_range(
    header_length: u64,
    data_offsets: (u64, u64),
) -> (u64, u64) {
    let payload_base = 8 + header_length;
    (payload_base + data_offsets.0, payload_base + data_offsets.1)
}

pub fn merge_byte_ranges(ranges: &[(u64, u64)], max_gap: u64) -> Vec<(u64, u64)> {
    if ranges.is_empty() {
        return Vec::new();
    }
    let mut ordered = ranges.to_vec();
    ordered.sort_by_key(|range| range.0);
    let mut merged = Vec::with_capacity(ordered.len());
    let mut current = ordered[0];
    for range in ordered.into_iter().skip(1) {
        if range.0 <= current.1.saturating_add(max_gap) {
            current.1 = current.1.max(range.1);
        } else {
            merged.push(current);
            current = range;
        }
    }
    merged.push(current);
    merged
}

pub fn default_merge_ranges(ranges: &[(u64, u64)]) -> Vec<(u64, u64)> {
    merge_byte_ranges(ranges, RANGE_MERGE_GAP_BYTES)
}

#[derive(Debug, Deserialize)]
struct RawTensorInfo {
    dtype: String,
    shape: Vec<u64>,
    data_offsets: Vec<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_header_length_and_tensors() {
        let json = br#"{"a":{"dtype":"F16","shape":[2,3],"data_offsets":[0,12]},"__metadata__":{"format":"pt"}}"#;
        let header_length = json.len() as u64;
        let mut prefix = header_length.to_le_bytes().to_vec();
        prefix.extend_from_slice(json);
        let parsed_len = parse_header_length(&prefix).expect("length");
        assert_eq!(parsed_len, header_length);
        let header = parse_safetensors_header(parsed_len, json).expect("header");
        assert_eq!(header.tensors.len(), 1);
        let tensor = header.tensors.get("a").expect("tensor a");
        assert_eq!(tensor.dtype, SafetensorsDtype::F16);
        assert_eq!(tensor.shape, vec![2, 3]);
        assert_eq!(
            tensor_payload_absolute_range(header_length, tensor.data_offsets),
            (8 + header_length, 8 + header_length + 12)
        );
        assert_eq!(header.metadata.get("format").map(String::as_str), Some("pt"));
    }

    #[test]
    fn merges_nearby_ranges() {
        let merged = merge_byte_ranges(&[(0, 10), (20, 30), (200_000, 200_010)], 64 * 1024);
        assert_eq!(merged, vec![(0, 30), (200_000, 200_010)]);
    }
}
