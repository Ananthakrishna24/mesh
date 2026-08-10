use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::{CapabilityReport, DeploymentId, NodeId, ReservationId, now_unix_ms};

pub const DEFAULT_HOLD_LEASE_MS: u64 = 60_000;
pub const DEFAULT_COMMIT_LEASE_MS: u64 = 30 * 60 * 1000;
pub const MAX_LEASE_MS: u64 = 2 * 60 * 60 * 1000;
pub const OFFER_TTL_MS: i64 = 15_000;
pub const MIN_LEASE_MS: u64 = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReservationState {
    Held,
    Committed,
}

impl ReservationState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Held => "held",
            Self::Committed => "committed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "held" => Some(Self::Held),
            "committed" => Some(Self::Committed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GpuResourceAmount {
    pub device_stable_id: String,
    pub memory_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ResourceAmount {
    pub system_memory_bytes: u64,
    pub disk_bytes: u64,
    pub execution_slots: u32,
    pub gpus: Vec<GpuResourceAmount>,
}

impl ResourceAmount {
    pub fn is_zero(&self) -> bool {
        self.system_memory_bytes == 0
            && self.disk_bytes == 0
            && self.execution_slots == 0
            && self.gpus.iter().all(|gpu| gpu.memory_bytes == 0)
    }

    pub fn saturate_min(&self, other: &Self) -> Self {
        let mut other_map: BTreeMap<&str, u64> = BTreeMap::new();
        for gpu in &other.gpus {
            other_map.insert(gpu.device_stable_id.as_str(), gpu.memory_bytes);
        }
        let mut gpus = Vec::new();
        let mut seen = BTreeMap::new();
        for gpu in &self.gpus {
            let available = other_map
                .get(gpu.device_stable_id.as_str())
                .copied()
                .unwrap_or(0);
            gpus.push(GpuResourceAmount {
                device_stable_id: gpu.device_stable_id.clone(),
                memory_bytes: gpu.memory_bytes.min(available),
            });
            seen.insert(gpu.device_stable_id.clone(), ());
        }
        for gpu in &other.gpus {
            if seen.contains_key(&gpu.device_stable_id) {
                continue;
            }
            gpus.push(GpuResourceAmount {
                device_stable_id: gpu.device_stable_id.clone(),
                memory_bytes: gpu.memory_bytes,
            });
        }
        gpus.sort_by(|left, right| left.device_stable_id.cmp(&right.device_stable_id));
        Self {
            system_memory_bytes: self.system_memory_bytes.min(other.system_memory_bytes),
            disk_bytes: self.disk_bytes.min(other.disk_bytes),
            execution_slots: self.execution_slots.min(other.execution_slots),
            gpus,
        }
    }

    pub fn fits_within(&self, available: &Self) -> Result<(), String> {
        if self.system_memory_bytes > available.system_memory_bytes {
            return Err(format!(
                "system memory {} exceeds available {}",
                self.system_memory_bytes, available.system_memory_bytes
            ));
        }
        if self.disk_bytes > available.disk_bytes {
            return Err(format!(
                "disk {} exceeds available {}",
                self.disk_bytes, available.disk_bytes
            ));
        }
        if self.execution_slots > available.execution_slots {
            return Err(format!(
                "execution slots {} exceed available {}",
                self.execution_slots, available.execution_slots
            ));
        }
        for gpu in &self.gpus {
            if gpu.memory_bytes == 0 {
                continue;
            }
            let free = available
                .gpus
                .iter()
                .find(|item| item.device_stable_id == gpu.device_stable_id)
                .map(|item| item.memory_bytes)
                .unwrap_or(0);
            if gpu.memory_bytes > free {
                return Err(format!(
                    "gpu {} memory {} exceeds available {}",
                    gpu.device_stable_id, gpu.memory_bytes, free
                ));
            }
        }
        Ok(())
    }

    pub fn checked_add(&self, other: &Self) -> Self {
        let mut map = BTreeMap::new();
        for gpu in self.gpus.iter().chain(other.gpus.iter()) {
            *map.entry(gpu.device_stable_id.clone()).or_insert(0u64) = map
                .get(&gpu.device_stable_id)
                .copied()
                .unwrap_or(0)
                .saturating_add(gpu.memory_bytes);
        }
        let gpus = map
            .into_iter()
            .map(|(device_stable_id, memory_bytes)| GpuResourceAmount {
                device_stable_id,
                memory_bytes,
            })
            .collect();
        Self {
            system_memory_bytes: self
                .system_memory_bytes
                .saturating_add(other.system_memory_bytes),
            disk_bytes: self.disk_bytes.saturating_add(other.disk_bytes),
            execution_slots: self.execution_slots.saturating_add(other.execution_slots),
            gpus,
        }
    }

    pub fn saturating_sub(&self, other: &Self) -> Self {
        let mut map = BTreeMap::new();
        for gpu in &self.gpus {
            map.insert(gpu.device_stable_id.clone(), gpu.memory_bytes);
        }
        for gpu in &other.gpus {
            let entry = map.entry(gpu.device_stable_id.clone()).or_insert(0);
            *entry = entry.saturating_sub(gpu.memory_bytes);
        }
        let gpus = map
            .into_iter()
            .map(|(device_stable_id, memory_bytes)| GpuResourceAmount {
                device_stable_id,
                memory_bytes,
            })
            .collect();
        Self {
            system_memory_bytes: self
                .system_memory_bytes
                .saturating_sub(other.system_memory_bytes),
            disk_bytes: self.disk_bytes.saturating_sub(other.disk_bytes),
            execution_slots: self.execution_slots.saturating_sub(other.execution_slots),
            gpus,
        }
    }

    pub fn summary_line(&self) -> String {
        let gpu = if self.gpus.is_empty() {
            "no GPU hold".to_owned()
        } else {
            self.gpus
                .iter()
                .filter(|gpu| gpu.memory_bytes > 0)
                .map(|gpu| {
                    format!(
                        "{}:{}",
                        gpu.device_stable_id,
                        crate::format_bytes(gpu.memory_bytes)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        format!(
            "RAM {} · disk {} · exec {} · {}",
            crate::format_bytes(self.system_memory_bytes),
            crate::format_bytes(self.disk_bytes),
            self.execution_slots,
            if gpu.is_empty() {
                "no GPU hold".to_owned()
            } else {
                gpu
            }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceCapacity {
    pub system_memory_bytes: u64,
    pub disk_bytes: u64,
    pub execution_slots: u32,
    pub gpus: Vec<GpuResourceAmount>,
    pub refreshed_at_unix_ms: i64,
}

impl ResourceCapacity {
    pub fn from_capability(report: &CapabilityReport) -> Self {
        let gpus = report
            .gpus
            .iter()
            .map(|gpu| GpuResourceAmount {
                device_stable_id: gpu.stable_id.clone(),
                memory_bytes: gpu.available_memory_bytes.unwrap_or(gpu.total_memory_bytes),
            })
            .collect::<Vec<_>>();
        let execution_slots = if gpus.is_empty() {
            1
        } else {
            gpus.len() as u32
        };
        Self {
            system_memory_bytes: report.memory_available_bytes,
            disk_bytes: report.disk_available_bytes,
            execution_slots,
            gpus,
            refreshed_at_unix_ms: report.collected_at_unix_ms,
        }
    }

    pub fn as_amount(&self) -> ResourceAmount {
        ResourceAmount {
            system_memory_bytes: self.system_memory_bytes,
            disk_bytes: self.disk_bytes,
            execution_slots: self.execution_slots,
            gpus: self.gpus.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceQuery {
    pub deployment_id: DeploymentId,
    pub requested: ResourceAmount,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceOffer {
    pub deployment_id: DeploymentId,
    pub available: ResourceAmount,
    pub offered: ResourceAmount,
    pub can_satisfy: bool,
    pub offer_expires_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReserveRequest {
    pub deployment_id: DeploymentId,
    pub reservation_id: ReservationId,
    pub amount: ResourceAmount,
    pub lease_duration_ms: u64,
    pub purpose: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReserveAccepted {
    pub deployment_id: DeploymentId,
    pub reservation_id: ReservationId,
    pub reserved: ResourceAmount,
    pub expires_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReserveRejected {
    pub deployment_id: DeploymentId,
    pub reservation_id: ReservationId,
    pub reason: String,
    pub available: ResourceAmount,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReservationCommit {
    pub deployment_id: DeploymentId,
    pub reservation_id: ReservationId,
    pub lease_duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReservationRelease {
    pub deployment_id: DeploymentId,
    pub reservation_id: ReservationId,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalReservation {
    pub reservation_id: ReservationId,
    pub deployment_id: DeploymentId,
    pub owner_node_id: NodeId,
    pub amount: ResourceAmount,
    pub state: ReservationState,
    pub purpose: String,
    pub expires_at_unix_ms: i64,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
}

impl LocalReservation {
    pub fn is_active(&self, now_unix_ms: i64) -> bool {
        self.expires_at_unix_ms > now_unix_ms
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReservationSummaryView {
    pub reservation_id: ReservationId,
    pub deployment_id: DeploymentId,
    pub owner_node_id: NodeId,
    pub state: ReservationState,
    pub purpose: String,
    pub amount_line: String,
    pub expires_at_unix_ms: i64,
}

impl ReservationSummaryView {
    pub fn from_reservation(reservation: &LocalReservation) -> Self {
        Self {
            reservation_id: reservation.reservation_id,
            deployment_id: reservation.deployment_id,
            owner_node_id: reservation.owner_node_id,
            state: reservation.state,
            purpose: reservation.purpose.clone(),
            amount_line: reservation.amount.summary_line(),
            expires_at_unix_ms: reservation.expires_at_unix_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ResourceManagerView {
    pub capacity_line: String,
    pub available_line: String,
    pub active: Vec<ReservationSummaryView>,
}

pub fn clamp_lease_ms(requested: u64, default_ms: u64) -> u64 {
    let value = if requested == 0 {
        default_ms
    } else {
        requested
    };
    value.clamp(MIN_LEASE_MS, MAX_LEASE_MS)
}

pub fn offer_expiry(now: i64) -> i64 {
    now.saturating_add(OFFER_TTL_MS)
}

pub fn current_time_ms() -> i64 {
    now_unix_ms()
}
