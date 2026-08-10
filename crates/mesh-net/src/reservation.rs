use bytes::Bytes;
use mesh_core::protocol::proto::{
    ControlEnvelope, GpuResourceAmount as ProtoGpuAmount, ReservationCommit as ProtoCommit,
    ReservationRelease as ProtoRelease, ReserveAccepted as ProtoAccepted,
    ReserveRejected as ProtoRejected, ReserveRequest as ProtoReserve,
    ResourceAmount as ProtoAmount, ResourceOffer as ProtoOffer, ResourceQuery as ProtoQuery,
    control_envelope::Body,
};
use mesh_core::{
    DeploymentId, GpuResourceAmount, LocalIdentity, PROTOCOL_MAJOR, PROTOCOL_MINOR,
    ReservationCommit, ReservationId, ReservationRelease, ReserveAccepted, ReserveRejected,
    ReserveRequest, ResourceAmount, ResourceOffer, ResourceQuery, random_message_id,
};

use crate::{NetError, NetResult};

pub fn build_resource_query_envelope(
    identity: &LocalIdentity,
    query: &ResourceQuery,
) -> ControlEnvelope {
    ControlEnvelope {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor: PROTOCOL_MINOR,
        message_id: random_message_id(),
        sender_node_id: identity.node_id.to_vec().into(),
        in_reply_to: None,
        body: Some(Body::ResourceQuery(ProtoQuery {
            deployment_id: Bytes::copy_from_slice(query.deployment_id.as_bytes()),
            requested: Some(amount_to_proto(&query.requested)),
        })),
    }
}

pub fn build_resource_offer_envelope(
    identity: &LocalIdentity,
    offer: &ResourceOffer,
    in_reply_to: Option<Bytes>,
) -> ControlEnvelope {
    ControlEnvelope {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor: PROTOCOL_MINOR,
        message_id: random_message_id(),
        sender_node_id: identity.node_id.to_vec().into(),
        in_reply_to,
        body: Some(Body::ResourceOffer(ProtoOffer {
            deployment_id: Bytes::copy_from_slice(offer.deployment_id.as_bytes()),
            available: Some(amount_to_proto(&offer.available)),
            offered: Some(amount_to_proto(&offer.offered)),
            can_satisfy: offer.can_satisfy,
            offer_expires_at_unix_ms: offer.offer_expires_at_unix_ms,
        })),
    }
}

pub fn build_reserve_request_envelope(
    identity: &LocalIdentity,
    request: &ReserveRequest,
) -> ControlEnvelope {
    ControlEnvelope {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor: PROTOCOL_MINOR,
        message_id: random_message_id(),
        sender_node_id: identity.node_id.to_vec().into(),
        in_reply_to: None,
        body: Some(Body::ReserveRequest(ProtoReserve {
            deployment_id: Bytes::copy_from_slice(request.deployment_id.as_bytes()),
            reservation_id: Bytes::copy_from_slice(request.reservation_id.as_bytes()),
            amount: Some(amount_to_proto(&request.amount)),
            lease_duration_ms: request.lease_duration_ms,
            purpose: request.purpose.clone(),
        })),
    }
}

pub fn build_reserve_accepted_envelope(
    identity: &LocalIdentity,
    accepted: &ReserveAccepted,
    in_reply_to: Option<Bytes>,
) -> ControlEnvelope {
    ControlEnvelope {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor: PROTOCOL_MINOR,
        message_id: random_message_id(),
        sender_node_id: identity.node_id.to_vec().into(),
        in_reply_to,
        body: Some(Body::ReserveAccepted(ProtoAccepted {
            deployment_id: Bytes::copy_from_slice(accepted.deployment_id.as_bytes()),
            reservation_id: Bytes::copy_from_slice(accepted.reservation_id.as_bytes()),
            reserved: Some(amount_to_proto(&accepted.reserved)),
            expires_at_unix_ms: accepted.expires_at_unix_ms,
        })),
    }
}

pub fn build_reserve_rejected_envelope(
    identity: &LocalIdentity,
    rejected: &ReserveRejected,
    in_reply_to: Option<Bytes>,
) -> ControlEnvelope {
    ControlEnvelope {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor: PROTOCOL_MINOR,
        message_id: random_message_id(),
        sender_node_id: identity.node_id.to_vec().into(),
        in_reply_to,
        body: Some(Body::ReserveRejected(ProtoRejected {
            deployment_id: Bytes::copy_from_slice(rejected.deployment_id.as_bytes()),
            reservation_id: Bytes::copy_from_slice(rejected.reservation_id.as_bytes()),
            reason: rejected.reason.clone(),
            available: Some(amount_to_proto(&rejected.available)),
        })),
    }
}

pub fn build_reservation_commit_envelope(
    identity: &LocalIdentity,
    commit: &ReservationCommit,
) -> ControlEnvelope {
    ControlEnvelope {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor: PROTOCOL_MINOR,
        message_id: random_message_id(),
        sender_node_id: identity.node_id.to_vec().into(),
        in_reply_to: None,
        body: Some(Body::ReservationCommit(ProtoCommit {
            deployment_id: Bytes::copy_from_slice(commit.deployment_id.as_bytes()),
            reservation_id: Bytes::copy_from_slice(commit.reservation_id.as_bytes()),
            lease_duration_ms: commit.lease_duration_ms,
        })),
    }
}

pub fn build_reservation_release_envelope(
    identity: &LocalIdentity,
    release: &ReservationRelease,
) -> ControlEnvelope {
    ControlEnvelope {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor: PROTOCOL_MINOR,
        message_id: random_message_id(),
        sender_node_id: identity.node_id.to_vec().into(),
        in_reply_to: None,
        body: Some(Body::ReservationRelease(ProtoRelease {
            deployment_id: Bytes::copy_from_slice(release.deployment_id.as_bytes()),
            reservation_id: Bytes::copy_from_slice(release.reservation_id.as_bytes()),
            reason: release.reason.clone(),
        })),
    }
}

pub fn resource_query_from_proto(query: ProtoQuery) -> NetResult<ResourceQuery> {
    Ok(ResourceQuery {
        deployment_id: deployment_id_from_bytes(&query.deployment_id)?,
        requested: amount_from_proto(query.requested.unwrap_or_default())?,
    })
}

pub fn resource_offer_from_proto(offer: ProtoOffer) -> NetResult<ResourceOffer> {
    Ok(ResourceOffer {
        deployment_id: deployment_id_from_bytes(&offer.deployment_id)?,
        available: amount_from_proto(offer.available.unwrap_or_default())?,
        offered: amount_from_proto(offer.offered.unwrap_or_default())?,
        can_satisfy: offer.can_satisfy,
        offer_expires_at_unix_ms: offer.offer_expires_at_unix_ms,
    })
}

pub fn reserve_request_from_proto(request: ProtoReserve) -> NetResult<ReserveRequest> {
    Ok(ReserveRequest {
        deployment_id: deployment_id_from_bytes(&request.deployment_id)?,
        reservation_id: reservation_id_from_bytes(&request.reservation_id)?,
        amount: amount_from_proto(request.amount.unwrap_or_default())?,
        lease_duration_ms: request.lease_duration_ms,
        purpose: request.purpose,
    })
}

pub fn reserve_accepted_from_proto(accepted: ProtoAccepted) -> NetResult<ReserveAccepted> {
    Ok(ReserveAccepted {
        deployment_id: deployment_id_from_bytes(&accepted.deployment_id)?,
        reservation_id: reservation_id_from_bytes(&accepted.reservation_id)?,
        reserved: amount_from_proto(accepted.reserved.unwrap_or_default())?,
        expires_at_unix_ms: accepted.expires_at_unix_ms,
    })
}

pub fn reserve_rejected_from_proto(rejected: ProtoRejected) -> NetResult<ReserveRejected> {
    Ok(ReserveRejected {
        deployment_id: deployment_id_from_bytes(&rejected.deployment_id)?,
        reservation_id: reservation_id_from_bytes(&rejected.reservation_id)?,
        reason: rejected.reason,
        available: amount_from_proto(rejected.available.unwrap_or_default())?,
    })
}

pub fn reservation_commit_from_proto(commit: ProtoCommit) -> NetResult<ReservationCommit> {
    Ok(ReservationCommit {
        deployment_id: deployment_id_from_bytes(&commit.deployment_id)?,
        reservation_id: reservation_id_from_bytes(&commit.reservation_id)?,
        lease_duration_ms: commit.lease_duration_ms,
    })
}

pub fn reservation_release_from_proto(release: ProtoRelease) -> NetResult<ReservationRelease> {
    Ok(ReservationRelease {
        deployment_id: deployment_id_from_bytes(&release.deployment_id)?,
        reservation_id: reservation_id_from_bytes(&release.reservation_id)?,
        reason: release.reason,
    })
}

fn amount_to_proto(amount: &ResourceAmount) -> ProtoAmount {
    ProtoAmount {
        system_memory_bytes: amount.system_memory_bytes,
        disk_bytes: amount.disk_bytes,
        execution_slots: amount.execution_slots,
        gpus: amount
            .gpus
            .iter()
            .map(|gpu| ProtoGpuAmount {
                device_stable_id: gpu.device_stable_id.clone(),
                memory_bytes: gpu.memory_bytes,
            })
            .collect(),
    }
}

fn amount_from_proto(amount: ProtoAmount) -> NetResult<ResourceAmount> {
    if amount.gpus.len() > 32 {
        return Err(NetError::Protocol(
            "resource amount lists too many gpus".to_owned(),
        ));
    }
    Ok(ResourceAmount {
        system_memory_bytes: amount.system_memory_bytes,
        disk_bytes: amount.disk_bytes,
        execution_slots: amount.execution_slots,
        gpus: amount
            .gpus
            .into_iter()
            .map(|gpu| {
                if gpu.device_stable_id.is_empty() || gpu.device_stable_id.len() > 128 {
                    return Err(NetError::Protocol(
                        "invalid gpu device stable id".to_owned(),
                    ));
                }
                Ok(GpuResourceAmount {
                    device_stable_id: gpu.device_stable_id,
                    memory_bytes: gpu.memory_bytes,
                })
            })
            .collect::<NetResult<Vec<_>>>()?,
    })
}

fn deployment_id_from_bytes(bytes: &[u8]) -> NetResult<DeploymentId> {
    DeploymentId::from_slice(bytes)
        .map_err(|error| NetError::Protocol(format!("invalid deployment id: {error}")))
}

fn reservation_id_from_bytes(bytes: &[u8]) -> NetResult<ReservationId> {
    ReservationId::from_slice(bytes)
        .map_err(|error| NetError::Protocol(format!("invalid reservation id: {error}")))
}
