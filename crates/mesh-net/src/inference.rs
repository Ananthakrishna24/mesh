use bytes::Bytes;
use mesh_core::protocol::proto::{
    CancelRequest as ProtoCancel, ControlEnvelope, InferenceRequest as ProtoInferenceRequest,
    NextTokenFeedback as ProtoNextToken, ReplicaStatus as ProtoReplicaStatus,
    TokenResult as ProtoTokenResult, control_envelope::Body,
};
use mesh_core::{
    DeploymentId, InferenceRequestSpec, LocalIdentity, NextTokenFeedback, PROTOCOL_MAJOR,
    PROTOCOL_MINOR, ReplicaEndpointView, RequestId, SamplingParams, StopReason, TokenResultEvent,
    random_message_id,
};

use crate::{NetError, NetResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaStatusMessage {
    pub deployment_id: DeploymentId,
    pub model_line: String,
    pub backend: String,
    pub ready: bool,
    pub healthy: bool,
    pub active_requests: u32,
    pub max_concurrent_requests: u32,
}

impl ReplicaStatusMessage {
    pub fn from_local_view(view: &ReplicaEndpointView) -> Result<Self, NetError> {
        Ok(Self {
            deployment_id: DeploymentId::parse_hex(&view.deployment_id)
                .map_err(|error| NetError::Protocol(error.to_string()))?,
            model_line: view.model_line.clone(),
            backend: view.backend.clone(),
            ready: view.ready,
            healthy: view.healthy,
            active_requests: view.active_requests,
            max_concurrent_requests: view.max_concurrent_requests,
        })
    }
}

pub fn build_replica_status_envelope(
    identity: &LocalIdentity,
    status: &ReplicaStatusMessage,
) -> ControlEnvelope {
    ControlEnvelope {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor: PROTOCOL_MINOR,
        message_id: random_message_id(),
        sender_node_id: identity.node_id.to_vec().into(),
        in_reply_to: None,
        body: Some(Body::ReplicaStatus(ProtoReplicaStatus {
            deployment_id: Bytes::copy_from_slice(status.deployment_id.as_bytes()),
            model_line: status.model_line.clone(),
            backend: status.backend.clone(),
            ready: status.ready,
            healthy: status.healthy,
            active_requests: status.active_requests,
            max_concurrent_requests: status.max_concurrent_requests,
        })),
    }
}

pub fn build_inference_request_envelope(
    identity: &LocalIdentity,
    request: &InferenceRequestSpec,
) -> ControlEnvelope {
    ControlEnvelope {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor: PROTOCOL_MINOR,
        message_id: random_message_id(),
        sender_node_id: identity.node_id.to_vec().into(),
        in_reply_to: None,
        body: Some(Body::InferenceRequest(ProtoInferenceRequest {
            deployment_id: Bytes::copy_from_slice(request.deployment_id.as_bytes()),
            request_id: Bytes::copy_from_slice(request.request_id.as_bytes()),
            input_token_ids: request.input_token_ids.clone(),
            max_new_tokens: request.sampling.max_new_tokens,
            temperature: request.sampling.temperature,
            top_k: request.sampling.top_k,
            top_p: request.sampling.top_p,
            repetition_penalty: request.sampling.repetition_penalty,
            presence_penalty: 0.0,
            frequency_penalty: 0.0,
            seed: request.sampling.seed,
            stop_token_ids: request.stop_token_ids.clone(),
            return_logprobs: request.return_logprobs,
        })),
    }
}

pub fn build_token_result_envelope(
    identity: &LocalIdentity,
    event: &TokenResultEvent,
) -> ControlEnvelope {
    ControlEnvelope {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor: PROTOCOL_MINOR,
        message_id: random_message_id(),
        sender_node_id: identity.node_id.to_vec().into(),
        in_reply_to: None,
        body: Some(Body::TokenResult(ProtoTokenResult {
            deployment_id: Bytes::copy_from_slice(event.deployment_id.as_bytes()),
            request_id: Bytes::copy_from_slice(event.request_id.as_bytes()),
            token_id: event.token_id,
            token_index: event.token_index,
            is_last: event.is_last,
            stop_reason: event.stop_reason.map(|reason| reason.as_str().to_owned()),
            sequence_length: event.sequence_length,
        })),
    }
}

pub fn build_cancel_request_envelope(
    identity: &LocalIdentity,
    deployment_id: DeploymentId,
    request_id: RequestId,
    reason: impl Into<String>,
) -> ControlEnvelope {
    ControlEnvelope {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor: PROTOCOL_MINOR,
        message_id: random_message_id(),
        sender_node_id: identity.node_id.to_vec().into(),
        in_reply_to: None,
        body: Some(Body::CancelRequest(ProtoCancel {
            deployment_id: Bytes::copy_from_slice(deployment_id.as_bytes()),
            request_id: Bytes::copy_from_slice(request_id.as_bytes()),
            reason: reason.into(),
        })),
    }
}

pub fn build_next_token_feedback_envelope(
    identity: &LocalIdentity,
    feedback: &NextTokenFeedback,
) -> ControlEnvelope {
    ControlEnvelope {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor: PROTOCOL_MINOR,
        message_id: random_message_id(),
        sender_node_id: identity.node_id.to_vec().into(),
        in_reply_to: None,
        body: Some(Body::NextTokenFeedback(ProtoNextToken {
            deployment_id: Bytes::copy_from_slice(feedback.deployment_id.as_bytes()),
            request_id: Bytes::copy_from_slice(feedback.request_id.as_bytes()),
            token_id: feedback.token_id,
            token_index: feedback.token_index,
            is_last: feedback.is_last,
        })),
    }
}

pub fn next_token_feedback_from_proto(feedback: ProtoNextToken) -> NetResult<NextTokenFeedback> {
    Ok(NextTokenFeedback {
        deployment_id: deployment_id_from_bytes(&feedback.deployment_id)?,
        request_id: request_id_from_bytes(&feedback.request_id)?,
        token_id: feedback.token_id,
        token_index: feedback.token_index,
        is_last: feedback.is_last,
    })
}

pub fn replica_status_from_proto(status: ProtoReplicaStatus) -> NetResult<ReplicaStatusMessage> {
    Ok(ReplicaStatusMessage {
        deployment_id: deployment_id_from_bytes(&status.deployment_id)?,
        model_line: status.model_line,
        backend: status.backend,
        ready: status.ready,
        healthy: status.healthy,
        active_requests: status.active_requests,
        max_concurrent_requests: status.max_concurrent_requests.max(1),
    })
}

pub fn inference_request_from_proto(
    request: ProtoInferenceRequest,
) -> NetResult<InferenceRequestSpec> {
    Ok(InferenceRequestSpec {
        deployment_id: deployment_id_from_bytes(&request.deployment_id)?,
        request_id: request_id_from_bytes(&request.request_id)?,
        input_token_ids: request.input_token_ids,
        sampling: SamplingParams {
            temperature: request.temperature,
            top_k: request.top_k,
            top_p: request.top_p,
            repetition_penalty: if request.repetition_penalty == 0.0 {
                1.0
            } else {
                request.repetition_penalty
            },
            seed: request.seed,
            max_new_tokens: request.max_new_tokens.max(1),
        },
        stop_token_ids: request.stop_token_ids,
        return_logprobs: request.return_logprobs,
    })
}

pub fn token_result_from_proto(result: ProtoTokenResult) -> NetResult<TokenResultEvent> {
    Ok(TokenResultEvent {
        deployment_id: deployment_id_from_bytes(&result.deployment_id)?,
        request_id: request_id_from_bytes(&result.request_id)?,
        token_id: result.token_id,
        token_index: result.token_index,
        is_last: result.is_last,
        stop_reason: result.stop_reason.as_deref().and_then(StopReason::parse),
        sequence_length: result.sequence_length,
    })
}

pub fn cancel_request_from_proto(
    cancel: ProtoCancel,
) -> NetResult<(DeploymentId, RequestId, String)> {
    Ok((
        deployment_id_from_bytes(&cancel.deployment_id)?,
        request_id_from_bytes(&cancel.request_id)?,
        cancel.reason,
    ))
}

fn deployment_id_from_bytes(bytes: &[u8]) -> NetResult<DeploymentId> {
    DeploymentId::from_slice(bytes).map_err(|error| NetError::Protocol(error.to_string()))
}

fn request_id_from_bytes(bytes: &[u8]) -> NetResult<RequestId> {
    RequestId::from_slice(bytes).map_err(|error| NetError::Protocol(error.to_string()))
}
