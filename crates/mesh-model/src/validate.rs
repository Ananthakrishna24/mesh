use crate::safetensors::{SafetensorsDtype, dtype_width_bytes};
use crate::{ModelError, ModelResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentRange {
    pub start: u64,
    pub end_inclusive: u64,
    pub total: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeValidation {
    pub requested_start: u64,
    pub requested_end_exclusive: u64,
    pub body_len: u64,
    pub content_range: ContentRange,
    pub expected_total: Option<u64>,
}

pub fn validate_content_range(input: &RangeValidation) -> ModelResult<()> {
    if input.content_range.end_inclusive < input.content_range.start {
        return Err(ModelError::Invalid(
            "content-range end before start".to_owned(),
        ));
    }
    let response_len = input
        .content_range
        .end_inclusive
        .saturating_sub(input.content_range.start)
        .saturating_add(1);
    if response_len != input.body_len {
        return Err(ModelError::Invalid(format!(
            "content-range length {response_len} != body length {}",
            input.body_len
        )));
    }
    if input.content_range.start != input.requested_start {
        return Err(ModelError::Invalid(format!(
            "content-range start {} != requested {}",
            input.content_range.start, input.requested_start
        )));
    }
    let requested_end_inclusive = input.requested_end_exclusive.saturating_sub(1);
    if input.requested_end_exclusive == 0
        || input.content_range.end_inclusive != requested_end_inclusive
    {
        return Err(ModelError::Invalid(format!(
            "content-range end {} != requested end {requested_end_inclusive}",
            input.content_range.end_inclusive
        )));
    }
    if let (Some(total), Some(expected)) = (input.content_range.total, input.expected_total) {
        if total != expected {
            return Err(ModelError::Invalid(format!(
                "content-range total {total} != expected artifact length {expected}"
            )));
        }
    }
    Ok(())
}

pub fn validate_tensor_byte_length(
    dtype: SafetensorsDtype,
    shape: &[u64],
    absolute_start: u64,
    absolute_end: u64,
) -> ModelResult<u64> {
    if absolute_end < absolute_start {
        return Err(ModelError::Invalid(
            "tensor absolute range end before start".to_owned(),
        ));
    }
    let actual = absolute_end - absolute_start;
    let mut expected = dtype_width_bytes(dtype);
    for dim in shape {
        expected = expected
            .checked_mul(*dim)
            .ok_or_else(|| ModelError::Invalid("tensor shape overflow".to_owned()))?;
    }
    if actual != expected {
        return Err(ModelError::Invalid(format!(
            "tensor byte length {actual} != dtype/shape product {expected}"
        )));
    }
    Ok(actual)
}

pub fn parse_content_range_header(value: &str) -> ModelResult<ContentRange> {
    let trimmed = value.trim();
    let rest = trimmed
        .strip_prefix("bytes ")
        .ok_or_else(|| ModelError::Invalid(format!("invalid content-range: {value}")))?;
    let (span, total_part) = rest
        .split_once('/')
        .ok_or_else(|| ModelError::Invalid(format!("invalid content-range: {value}")))?;
    let (start_text, end_text) = span
        .split_once('-')
        .ok_or_else(|| ModelError::Invalid(format!("invalid content-range span: {value}")))?;
    let start = start_text.parse::<u64>().map_err(|_| {
        ModelError::Invalid(format!("invalid content-range start: {value}"))
    })?;
    let end_inclusive = end_text.parse::<u64>().map_err(|_| {
        ModelError::Invalid(format!("invalid content-range end: {value}"))
    })?;
    let total = if total_part == "*" {
        None
    } else {
        Some(total_part.parse::<u64>().map_err(|_| {
            ModelError::Invalid(format!("invalid content-range total: {value}"))
        })?)
    };
    Ok(ContentRange {
        start,
        end_inclusive,
        total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_matching_content_range() {
        let range = parse_content_range_header("bytes 10-19/100").expect("parse");
        validate_content_range(&RangeValidation {
            requested_start: 10,
            requested_end_exclusive: 20,
            body_len: 10,
            content_range: range,
            expected_total: Some(100),
        })
        .expect("valid");
    }

    #[test]
    fn rejects_dtype_shape_mismatch() {
        let error = validate_tensor_byte_length(SafetensorsDtype::F16, &[2, 3], 0, 10)
            .expect_err("mismatch");
        assert!(error.to_string().contains("byte length"));
    }
}
