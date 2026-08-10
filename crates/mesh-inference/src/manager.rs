use std::collections::HashMap;

use mesh_core::{
    CapabilityReport, DEFAULT_COMMIT_LEASE_MS, DEFAULT_HOLD_LEASE_MS, DeploymentId, LocalReservation,
    NodeId, ReservationId, ReservationState, ReservationSummaryView, ReserveAccepted,
    ReserveRejected, ResourceAmount, ResourceCapacity, ResourceManagerView, ResourceOffer,
    ResourceQuery, clamp_lease_ms, now_unix_ms, offer_expiry,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReserveOutcome {
    Accepted(ReserveAccepted),
    Rejected(ReserveRejected),
}

#[derive(Debug, Clone)]
pub struct LocalResourceManager {
    capacity: ResourceCapacity,
    reservations: HashMap<ReservationId, LocalReservation>,
}

impl LocalResourceManager {
    pub fn new(capacity: ResourceCapacity) -> Self {
        Self {
            capacity,
            reservations: HashMap::new(),
        }
    }

    pub fn from_capability(report: &CapabilityReport) -> Self {
        Self::new(ResourceCapacity::from_capability(report))
    }

    pub fn restore(capacity: ResourceCapacity, reservations: Vec<LocalReservation>) -> Self {
        let mut manager = Self::new(capacity);
        let now = now_unix_ms();
        for reservation in reservations {
            if reservation.is_active(now) {
                manager
                    .reservations
                    .insert(reservation.reservation_id, reservation);
            }
        }
        manager
    }

    pub fn refresh_capacity(&mut self, report: &CapabilityReport) {
        self.capacity = ResourceCapacity::from_capability(report);
        self.expire_due(now_unix_ms());
    }

    pub fn capacity(&self) -> &ResourceCapacity {
        &self.capacity
    }

    pub fn reservations(&self) -> impl Iterator<Item = &LocalReservation> {
        self.reservations.values()
    }

    pub fn active_reservations(&self) -> Vec<LocalReservation> {
        let now = now_unix_ms();
        let mut items = self
            .reservations
            .values()
            .filter(|item| item.is_active(now))
            .cloned()
            .collect::<Vec<_>>();
        items.sort_by(|left, right| left.created_at_unix_ms.cmp(&right.created_at_unix_ms));
        items
    }

    pub fn view(&self) -> ResourceManagerView {
        let now = now_unix_ms();
        let available = self.available_amount(now);
        let active = self
            .active_reservations()
            .into_iter()
            .map(|item| ReservationSummaryView::from_reservation(&item))
            .collect();
        ResourceManagerView {
            capacity_line: self.capacity.as_amount().summary_line(),
            available_line: available.summary_line(),
            active,
        }
    }

    pub fn expire_due(&mut self, now: i64) -> Vec<LocalReservation> {
        let expired_ids = self
            .reservations
            .iter()
            .filter(|(_, reservation)| reservation.expires_at_unix_ms <= now)
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        let mut expired = Vec::with_capacity(expired_ids.len());
        for id in expired_ids {
            if let Some(item) = self.reservations.remove(&id) {
                expired.push(item);
            }
        }
        expired
    }

    pub fn offer(&mut self, query: &ResourceQuery) -> ResourceOffer {
        let now = now_unix_ms();
        self.expire_due(now);
        let available = self.available_amount(now);
        let offered = if query.requested.is_zero() {
            available.clone()
        } else {
            query.requested.saturate_min(&available)
        };
        let can_satisfy = if query.requested.is_zero() {
            true
        } else {
            query.requested.fits_within(&available).is_ok()
        };
        ResourceOffer {
            deployment_id: query.deployment_id,
            available,
            offered,
            can_satisfy,
            offer_expires_at_unix_ms: offer_expiry(now),
        }
    }

    pub fn reserve(
        &mut self,
        owner_node_id: NodeId,
        request: &mesh_core::ReserveRequest,
    ) -> ReserveOutcome {
        let now = now_unix_ms();
        self.expire_due(now);

        if let Some(existing) = self.reservations.get(&request.reservation_id).cloned() {
            if existing.owner_node_id == owner_node_id
                && existing.deployment_id == request.deployment_id
                && existing.amount == request.amount
                && existing.is_active(now)
            {
                return ReserveOutcome::Accepted(ReserveAccepted {
                    deployment_id: existing.deployment_id,
                    reservation_id: existing.reservation_id,
                    reserved: existing.amount,
                    expires_at_unix_ms: existing.expires_at_unix_ms,
                });
            }
            return ReserveOutcome::Rejected(ReserveRejected {
                deployment_id: request.deployment_id,
                reservation_id: request.reservation_id,
                reason: "reservation id already used with different parameters".to_owned(),
                available: self.available_amount(now),
            });
        }

        if request.amount.is_zero() {
            return ReserveOutcome::Rejected(ReserveRejected {
                deployment_id: request.deployment_id,
                reservation_id: request.reservation_id,
                reason: "reservation amount must be greater than zero".to_owned(),
                available: self.available_amount(now),
            });
        }

        let available = self.available_amount(now);
        if let Err(reason) = request.amount.fits_within(&available) {
            return ReserveOutcome::Rejected(ReserveRejected {
                deployment_id: request.deployment_id,
                reservation_id: request.reservation_id,
                reason,
                available,
            });
        }

        let lease_ms = clamp_lease_ms(request.lease_duration_ms, DEFAULT_HOLD_LEASE_MS);
        let expires_at_unix_ms = now.saturating_add(lease_ms as i64);
        let purpose = if request.purpose.trim().is_empty() {
            "unspecified".to_owned()
        } else {
            request.purpose.chars().take(128).collect()
        };
        let reservation = LocalReservation {
            reservation_id: request.reservation_id,
            deployment_id: request.deployment_id,
            owner_node_id,
            amount: request.amount.clone(),
            state: ReservationState::Held,
            purpose,
            expires_at_unix_ms,
            created_at_unix_ms: now,
            updated_at_unix_ms: now,
        };
        self.reservations
            .insert(reservation.reservation_id, reservation.clone());
        ReserveOutcome::Accepted(ReserveAccepted {
            deployment_id: reservation.deployment_id,
            reservation_id: reservation.reservation_id,
            reserved: reservation.amount,
            expires_at_unix_ms: reservation.expires_at_unix_ms,
        })
    }

    pub fn commit(
        &mut self,
        owner_node_id: NodeId,
        deployment_id: DeploymentId,
        reservation_id: ReservationId,
        lease_duration_ms: u64,
    ) -> Result<LocalReservation, String> {
        let now = now_unix_ms();
        self.expire_due(now);
        let Some(reservation) = self.reservations.get_mut(&reservation_id) else {
            return Err("unknown reservation".to_owned());
        };
        if reservation.owner_node_id != owner_node_id {
            return Err("reservation owned by another coordinator".to_owned());
        }
        if reservation.deployment_id != deployment_id {
            return Err("deployment id mismatch".to_owned());
        }
        if !reservation.is_active(now) {
            self.reservations.remove(&reservation_id);
            return Err("reservation expired".to_owned());
        }
        let lease_ms = clamp_lease_ms(lease_duration_ms, DEFAULT_COMMIT_LEASE_MS);
        reservation.state = ReservationState::Committed;
        reservation.expires_at_unix_ms = now.saturating_add(lease_ms as i64);
        reservation.updated_at_unix_ms = now;
        Ok(reservation.clone())
    }

    pub fn release(
        &mut self,
        owner_node_id: Option<NodeId>,
        deployment_id: Option<DeploymentId>,
        reservation_id: ReservationId,
    ) -> Result<LocalReservation, String> {
        let Some(reservation) = self.reservations.get(&reservation_id).cloned() else {
            return Err("unknown reservation".to_owned());
        };
        if let Some(owner) = owner_node_id {
            if reservation.owner_node_id != owner {
                return Err("reservation owned by another coordinator".to_owned());
            }
        }
        if let Some(deployment) = deployment_id {
            if reservation.deployment_id != deployment {
                return Err("deployment id mismatch".to_owned());
            }
        }
        self.reservations.remove(&reservation_id);
        Ok(reservation)
    }

    pub fn release_owner(&mut self, owner_node_id: NodeId) -> Vec<LocalReservation> {
        let ids = self
            .reservations
            .iter()
            .filter(|(_, item)| item.owner_node_id == owner_node_id)
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        let mut released = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(item) = self.reservations.remove(&id) {
                released.push(item);
            }
        }
        released
    }

    pub fn release_all(&mut self) -> Vec<LocalReservation> {
        let items = self.reservations.values().cloned().collect::<Vec<_>>();
        self.reservations.clear();
        items
    }

    pub fn available_amount(&self, now: i64) -> ResourceAmount {
        let mut reserved = ResourceAmount::default();
        for reservation in self.reservations.values() {
            if reservation.is_active(now) {
                reserved = reserved.checked_add(&reservation.amount);
            }
        }
        self.capacity.as_amount().saturating_sub(&reserved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesh_core::{GpuResourceAmount, ReserveRequest};
    fn capacity() -> ResourceCapacity {
        ResourceCapacity {
            system_memory_bytes: 8 * 1024 * 1024 * 1024,
            disk_bytes: 100 * 1024 * 1024 * 1024,
            execution_slots: 2,
            gpus: vec![GpuResourceAmount {
                device_stable_id: "gpu0".to_owned(),
                memory_bytes: 12 * 1024 * 1024 * 1024,
            }],
            refreshed_at_unix_ms: now_unix_ms(),
        }
    }

    fn amount(gpu_bytes: u64) -> ResourceAmount {
        ResourceAmount {
            system_memory_bytes: 1024 * 1024 * 1024,
            disk_bytes: 2 * 1024 * 1024 * 1024,
            execution_slots: 1,
            gpus: vec![GpuResourceAmount {
                device_stable_id: "gpu0".to_owned(),
                memory_bytes: gpu_bytes,
            }],
        }
    }

    #[test]
    fn two_coordinators_cannot_reserve_same_capacity() {
        let mut manager = LocalResourceManager::new(capacity());
        let owner_a = NodeId::from_bytes([1; 32]);
        let owner_b = NodeId::from_bytes([2; 32]);
        let deployment_a = DeploymentId::from_bytes([3; 16]);
        let deployment_b = DeploymentId::from_bytes([4; 16]);
        let request_a = ReserveRequest {
            deployment_id: deployment_a,
            reservation_id: ReservationId::from_bytes([5; 16]),
            amount: amount(10 * 1024 * 1024 * 1024),
            lease_duration_ms: DEFAULT_HOLD_LEASE_MS,
            purpose: "coord-a".to_owned(),
        };
        let request_b = ReserveRequest {
            deployment_id: deployment_b,
            reservation_id: ReservationId::from_bytes([6; 16]),
            amount: amount(10 * 1024 * 1024 * 1024),
            lease_duration_ms: DEFAULT_HOLD_LEASE_MS,
            purpose: "coord-b".to_owned(),
        };

        match manager.reserve(owner_a, &request_a) {
            ReserveOutcome::Accepted(_) => {}
            ReserveOutcome::Rejected(rejected) => panic!("first reserve failed: {rejected:?}"),
        }
        match manager.reserve(owner_b, &request_b) {
            ReserveOutcome::Rejected(rejected) => {
                assert!(rejected.reason.contains("gpu0"));
            }
            ReserveOutcome::Accepted(accepted) => {
                panic!("second coordinator should fail, got {accepted:?}")
            }
        }
        assert_eq!(manager.active_reservations().len(), 1);
    }

    #[test]
    fn commit_and_release_free_capacity() {
        let mut manager = LocalResourceManager::new(capacity());
        let owner = NodeId::from_bytes([9; 32]);
        let deployment = DeploymentId::from_bytes([8; 16]);
        let reservation_id = ReservationId::from_bytes([7; 16]);
        let request = ReserveRequest {
            deployment_id: deployment,
            reservation_id,
            amount: amount(4 * 1024 * 1024 * 1024),
            lease_duration_ms: DEFAULT_HOLD_LEASE_MS,
            purpose: "stage".to_owned(),
        };
        assert!(matches!(
            manager.reserve(owner, &request),
            ReserveOutcome::Accepted(_)
        ));
        let committed = manager
            .commit(owner, deployment, reservation_id, DEFAULT_COMMIT_LEASE_MS)
            .expect("commit");
        assert_eq!(committed.state, ReservationState::Committed);
        manager
            .release(Some(owner), Some(deployment), reservation_id)
            .expect("release");
        let available = manager.available_amount(now_unix_ms());
        assert_eq!(available.execution_slots, capacity().execution_slots);
        assert_eq!(
            available.gpus[0].memory_bytes,
            12 * 1024 * 1024 * 1024
        );
    }

    #[test]
    fn offer_reports_remaining_capacity() {
        let mut manager = LocalResourceManager::new(capacity());
        let owner = NodeId::from_bytes([11; 32]);
        let deployment = DeploymentId::from_bytes([12; 16]);
        let request = ReserveRequest {
            deployment_id: deployment,
            reservation_id: ReservationId::from_bytes([13; 16]),
            amount: amount(8 * 1024 * 1024 * 1024),
            lease_duration_ms: DEFAULT_HOLD_LEASE_MS,
            purpose: "hold".to_owned(),
        };
        assert!(matches!(
            manager.reserve(owner, &request),
            ReserveOutcome::Accepted(_)
        ));
        let offer = manager.offer(&ResourceQuery {
            deployment_id: DeploymentId::from_bytes([14; 16]),
            requested: amount(8 * 1024 * 1024 * 1024),
        });
        assert!(!offer.can_satisfy);
        assert!(offer.offered.gpus[0].memory_bytes < 8 * 1024 * 1024 * 1024);
    }
}
