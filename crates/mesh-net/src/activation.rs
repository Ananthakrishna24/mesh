use mesh_core::{
    ActivationHeader, ActivationValidationError, DeploymentId, RequestId,
    ACTIVATION_HEADER_BYTES, ACTIVATION_MAX_IN_FLIGHT_PER_STAGE_REQUEST,
    ACTIVATION_MAX_PAYLOAD_BYTES,
};
use quinn::{Connection, RecvStream, SendStream};

use crate::{NetError, NetResult};

#[derive(Debug, Clone)]
pub struct ActivationFrame {
    pub header: ActivationHeader,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct ActivationReceiveContext {
    pub deployment_id: DeploymentId,
    pub request_id: RequestId,
    pub expected_destination_stage: u16,
    pub expected_source_stage: u16,
    pub max_payload_bytes: u64,
    pub seen_transfer_ids: Vec<u64>,
    pub queued_count: u32,
}

impl ActivationReceiveContext {
    pub fn new(
        deployment_id: DeploymentId,
        request_id: RequestId,
        expected_source_stage: u16,
        expected_destination_stage: u16,
    ) -> Self {
        Self {
            deployment_id,
            request_id,
            expected_destination_stage,
            expected_source_stage,
            max_payload_bytes: ACTIVATION_MAX_PAYLOAD_BYTES,
            seen_transfer_ids: Vec::new(),
            queued_count: 0,
        }
    }
}

pub async fn write_activation_frame(
    send: &mut SendStream,
    header: &ActivationHeader,
    payload: &[u8],
) -> NetResult<()> {
    header
        .validate_shape_only()
        .map_err(activation_error_to_net)?;
    if payload.len() as u64 != header.payload_len {
        return Err(NetError::Protocol(format!(
            "activation payload len {} != header {}",
            payload.len(),
            header.payload_len
        )));
    }
    let encoded = header.encode().map_err(activation_error_to_net)?;
    send.write_all(&encoded).await?;
    send.write_all(payload).await?;
    send.finish().map_err(|error| NetError::Protocol(error.to_string()))?;
    Ok(())
}

pub async fn send_activation_on_connection(
    connection: &Connection,
    header: &ActivationHeader,
    payload: &[u8],
) -> NetResult<()> {
    let mut send = connection
        .open_uni()
        .await
        .map_err(|error| NetError::Protocol(error.to_string()))?;
    write_activation_frame(&mut send, header, payload).await
}

pub async fn read_activation_frame(recv: &mut RecvStream) -> NetResult<ActivationFrame> {
    let mut header_bytes = [0u8; ACTIVATION_HEADER_BYTES];
    recv.read_exact(&mut header_bytes)
        .await
        .map_err(|error| NetError::Protocol(error.to_string()))?;
    let header = ActivationHeader::decode(&header_bytes).map_err(activation_error_to_net)?;
    if header.payload_len > ACTIVATION_MAX_PAYLOAD_BYTES {
        return Err(activation_error_to_net(
            ActivationValidationError::MalformedFrame("payload exceeds protocol maximum"),
        ));
    }
    let mut payload = vec![0u8; header.payload_len as usize];
    if !payload.is_empty() {
        recv.read_exact(&mut payload)
            .await
            .map_err(|error| NetError::Protocol(error.to_string()))?;
    }
    let mut trailer = [0u8; 1];
    match recv.read_exact(&mut trailer).await {
        Ok(()) => {
            return Err(activation_error_to_net(
                ActivationValidationError::MalformedFrame("activation stream longer than payload"),
            ));
        }
        Err(quinn::ReadExactError::FinishedEarly(_)) => {}
        Err(error) => return Err(NetError::Protocol(error.to_string())),
    }
    Ok(ActivationFrame { header, payload })
}

pub fn validate_activation_for_request(
    frame: &ActivationFrame,
    ctx: &mut ActivationReceiveContext,
) -> NetResult<()> {
    let header = &frame.header;
    header
        .validate_shape_only()
        .map_err(activation_error_to_net)?;
    if header.deployment_id != ctx.deployment_id || header.request_id != ctx.request_id {
        return Err(activation_error_to_net(
            ActivationValidationError::InvalidState("deployment/request mismatch"),
        ));
    }
    if header.source_stage != ctx.expected_source_stage
        || header.destination_stage != ctx.expected_destination_stage
    {
        return Err(activation_error_to_net(
            ActivationValidationError::InvalidState("stage index mismatch"),
        ));
    }
    if ctx.seen_transfer_ids.contains(&header.transfer_id) {
        return Err(activation_error_to_net(
            ActivationValidationError::MalformedFrame("duplicate transfer id"),
        ));
    }
    if header.payload_len > ctx.max_payload_bytes {
        return Err(activation_error_to_net(
            ActivationValidationError::MalformedFrame("payload exceeds deployment limit"),
        ));
    }
    if frame.payload.len() as u64 != header.payload_len {
        return Err(activation_error_to_net(
            ActivationValidationError::MalformedFrame("payload byte count mismatch"),
        ));
    }
    if ctx.queued_count >= ACTIVATION_MAX_IN_FLIGHT_PER_STAGE_REQUEST {
        return Err(activation_error_to_net(
            ActivationValidationError::ResourceBusy("activation queue full"),
        ));
    }
    ctx.seen_transfer_ids.push(header.transfer_id);
    ctx.queued_count = ctx.queued_count.saturating_add(1);
    Ok(())
}


fn activation_error_to_net(error: ActivationValidationError) -> NetError {
    NetError::Protocol(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesh_core::{DeploymentId, RequestId, TransferKind};

    #[test]
    fn validate_accepts_first_frame_and_rejects_duplicate() {
        let deployment_id = DeploymentId::from_bytes([9; 16]);
        let request_id = RequestId::from_bytes([8; 16]);
        let header = ActivationHeader::qwen3_hidden(
            deployment_id,
            request_id,
            1,
            0,
            1,
            TransferKind::Prefill,
            1,
            2,
            16,
            0,
        )
        .unwrap();
        let payload = vec![0u8; header.payload_len as usize];
        let frame = ActivationFrame { header, payload };
        let mut ctx = ActivationReceiveContext::new(deployment_id, request_id, 0, 1);
        validate_activation_for_request(&frame, &mut ctx).unwrap();
        let err = validate_activation_for_request(&frame, &mut ctx).unwrap_err();
        assert!(err.to_string().contains("duplicate transfer id"));
    }
}
