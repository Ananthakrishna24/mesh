use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter, Result as FmtResult};
use std::net::{IpAddr, SocketAddr};

use crate::{EnrollmentId, LinkMeasurement, NodeId, now_unix_ms};

pub const MAX_PEER_CANDIDATES: usize = 32;

const LIFETIME_30_MIN_MS: i64 = 30 * 60 * 1000;
const LIFETIME_2_HOUR_MS: i64 = 2 * 60 * 60 * 1000;
const LIFETIME_24_HOUR_MS: i64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CandidateKind {
    GlobalIpv6,
    PublicIpv4,
    RouterMapping,
    Manual,
    PeerObserved,
    LocalNetwork,
}

impl CandidateKind {
    pub fn priority(self) -> u16 {
        match self {
            Self::GlobalIpv6 => 100,
            Self::PublicIpv4 => 90,
            Self::RouterMapping => 80,
            Self::Manual => 70,
            Self::PeerObserved => 60,
            Self::LocalNetwork => 50,
        }
    }

    pub fn default_lifetime_ms(self) -> Option<i64> {
        match self {
            Self::GlobalIpv6 | Self::PublicIpv4 | Self::LocalNetwork => Some(LIFETIME_24_HOUR_MS),
            Self::RouterMapping => Some(LIFETIME_2_HOUR_MS),
            Self::PeerObserved => Some(LIFETIME_30_MIN_MS),
            Self::Manual => None,
        }
    }

    pub fn default_expiry(self, observed_at_unix_ms: i64) -> Option<i64> {
        self.default_lifetime_ms()
            .map(|lifetime| observed_at_unix_ms.saturating_add(lifetime))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CandidateReachability {
    #[default]
    Unknown,
    Reachable,
    Unreachable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PeerRecordOrigin {
    LocalSelf,
    #[default]
    DirectPeer,
    IndirectPeer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointCandidate {
    pub kind: CandidateKind,
    pub address: SocketAddr,
    pub priority: u16,
    pub observed_at_unix_ms: i64,
    pub expires_at_unix_ms: Option<i64>,
    pub source_node_id: Option<NodeId>,
    pub reachability: CandidateReachability,
}

impl EndpointCandidate {
    pub fn new(kind: CandidateKind, address: SocketAddr) -> Self {
        Self::new_at(kind, address, now_unix_ms())
    }

    pub fn new_at(kind: CandidateKind, address: SocketAddr, observed_at_unix_ms: i64) -> Self {
        Self {
            kind,
            address: normalize_candidate_address(address),
            priority: kind.priority(),
            observed_at_unix_ms,
            expires_at_unix_ms: kind.default_expiry(observed_at_unix_ms),
            source_node_id: None,
            reachability: CandidateReachability::Unknown,
        }
    }

    pub fn with_source(mut self, source_node_id: NodeId) -> Self {
        self.source_node_id = Some(source_node_id);
        self
    }

    pub fn with_expiry(mut self, expires_at_unix_ms: Option<i64>) -> Self {
        self.expires_at_unix_ms = expires_at_unix_ms;
        self
    }

    pub fn with_reachability(mut self, reachability: CandidateReachability) -> Self {
        self.reachability = reachability;
        self
    }

    pub fn is_expired(&self, now_ms: i64) -> bool {
        self.expires_at_unix_ms
            .is_some_and(|expires| expires <= now_ms)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerRecord {
    pub node_id: NodeId,
    pub display_name: String,
    pub certificate_der: Vec<u8>,
    pub candidates: Vec<EndpointCandidate>,
    pub last_successful_address: Option<SocketAddr>,
    pub last_seen_unix_ms: Option<i64>,
    pub first_seen_unix_ms: i64,
    pub record_updated_at_unix_ms: i64,
    pub origin: PeerRecordOrigin,
}

impl PeerRecord {
    pub fn new(
        node_id: NodeId,
        display_name: String,
        certificate_der: Vec<u8>,
        candidates: Vec<EndpointCandidate>,
    ) -> Self {
        let now = now_unix_ms();
        Self {
            node_id,
            display_name,
            certificate_der,
            candidates,
            last_successful_address: None,
            last_seen_unix_ms: None,
            first_seen_unix_ms: now,
            record_updated_at_unix_ms: now,
            origin: PeerRecordOrigin::DirectPeer,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PeerSummary {
    pub node_id: NodeId,
    pub display_name: String,
    pub connected: bool,
    pub address: Option<SocketAddr>,
    pub hardware_line: Option<String>,
    pub link: Option<LinkMeasurement>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InvitationState {
    Pending,
    Bound,
    Consumed,
    Expired,
}

impl Display for InvitationState {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Bound => write!(f, "bound"),
            Self::Consumed => write!(f, "consumed"),
            Self::Expired => write!(f, "expired"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvitationRecord {
    pub enrollment_id: EnrollmentId,
    pub secret_digest: [u8; 32],
    pub expires_at_unix_ms: i64,
    pub state: InvitationState,
    pub bound_node_id: Option<NodeId>,
    pub created_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerMergeError {
    CertificateMismatch,
    NodeIdMismatch,
    SelfRecordFromNetwork,
}

impl Display for PeerMergeError {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::CertificateMismatch => write!(f, "peer certificate does not match node id"),
            Self::NodeIdMismatch => write!(f, "cannot merge peers with different node ids"),
            Self::SelfRecordFromNetwork => write!(f, "refusing network update for local node id"),
        }
    }
}

pub fn normalize_candidate_address(address: SocketAddr) -> SocketAddr {
    match address {
        SocketAddr::V6(v6) => {
            if let Some(v4) = v6.ip().to_ipv4_mapped() {
                SocketAddr::new(IpAddr::V4(v4), v6.port())
            } else {
                address
            }
        }
        other => other,
    }
}

pub fn candidate_is_advertisable(candidate: &EndpointCandidate, now_ms: i64) -> bool {
    if candidate.is_expired(now_ms) || candidate.address.port() == 0 {
        return false;
    }
    match candidate.address.ip() {
        IpAddr::V4(ip) => !ip.is_unspecified(),
        IpAddr::V6(ip) => !ip.is_unspecified(),
    }
}

pub fn filter_advertised_candidates(
    candidates: &[EndpointCandidate],
    now_ms: i64,
) -> Vec<EndpointCandidate> {
    let mut out = candidates
        .iter()
        .filter(|candidate| candidate_is_advertisable(candidate, now_ms))
        .cloned()
        .collect::<Vec<_>>();
    sort_candidates_for_dial(&mut out);
    out.truncate(MAX_PEER_CANDIDATES);
    out
}

pub fn sort_candidates_for_dial(candidates: &mut [EndpointCandidate]) {
    candidates.sort_by(|left, right| {
        right
            .kind
            .priority()
            .cmp(&left.kind.priority())
            .then_with(|| right.priority.cmp(&left.priority))
            .then_with(|| right.observed_at_unix_ms.cmp(&left.observed_at_unix_ms))
            .then_with(|| left.address.to_string().cmp(&right.address.to_string()))
    });
}

pub fn merge_candidates(
    local: &[EndpointCandidate],
    incoming: &[EndpointCandidate],
    now_ms: i64,
    allow_remote_reachability_upgrade: bool,
) -> Vec<EndpointCandidate> {
    let mut by_address: BTreeMap<SocketAddr, EndpointCandidate> = BTreeMap::new();

    for candidate in local.iter().chain(incoming.iter()) {
        if candidate.is_expired(now_ms) {
            continue;
        }
        let address = normalize_candidate_address(candidate.address);
        let mut next = candidate.clone();
        next.address = address;
        match by_address.get(&address) {
            None => {
                by_address.insert(address, next);
            }
            Some(existing) => {
                by_address.insert(
                    address,
                    pick_candidate(existing, &next, allow_remote_reachability_upgrade),
                );
            }
        }
    }

    let mut merged = by_address.into_values().collect::<Vec<_>>();
    trim_candidates(&mut merged, now_ms);
    sort_candidates_for_dial(&mut merged);
    merged
}

pub fn merge_peer_records(
    local: Option<&PeerRecord>,
    incoming: &PeerRecord,
    local_node_id: NodeId,
    now_ms: i64,
    from_direct_subject: bool,
) -> Result<PeerRecord, PeerMergeError> {
    if incoming.node_id == local_node_id {
        return Err(PeerMergeError::SelfRecordFromNetwork);
    }
    if NodeId::from_certificate_der(&incoming.certificate_der) != incoming.node_id {
        return Err(PeerMergeError::CertificateMismatch);
    }

    let Some(local) = local else {
        let mut created = incoming.clone();
        created.candidates =
            merge_candidates(&[], &incoming.candidates, now_ms, from_direct_subject);
        created.first_seen_unix_ms = if created.first_seen_unix_ms == 0 {
            now_ms
        } else {
            created.first_seen_unix_ms
        };
        created.record_updated_at_unix_ms = now_ms;
        created.origin = if from_direct_subject {
            PeerRecordOrigin::DirectPeer
        } else {
            PeerRecordOrigin::IndirectPeer
        };
        if from_direct_subject {
            created.last_seen_unix_ms = Some(now_ms);
        }
        return Ok(created);
    };

    if local.node_id != incoming.node_id {
        return Err(PeerMergeError::NodeIdMismatch);
    }
    if local.certificate_der != incoming.certificate_der {
        return Err(PeerMergeError::CertificateMismatch);
    }

    let mut merged = local.clone();
    let subject_name_wins = from_direct_subject
        || incoming.record_updated_at_unix_ms >= local.record_updated_at_unix_ms;
    if subject_name_wins && !incoming.display_name.is_empty() {
        merged.display_name = incoming.display_name.clone();
    }

    merged.candidates = merge_candidates(
        &local.candidates,
        &incoming.candidates,
        now_ms,
        from_direct_subject,
    );

    if from_direct_subject {
        merged.last_seen_unix_ms = Some(
            local
                .last_seen_unix_ms
                .unwrap_or(0)
                .max(now_ms)
                .max(incoming.last_seen_unix_ms.unwrap_or(0)),
        );
        merged.origin = PeerRecordOrigin::DirectPeer;
    } else if merged.origin != PeerRecordOrigin::DirectPeer
        && merged.origin != PeerRecordOrigin::LocalSelf
    {
        merged.origin = PeerRecordOrigin::IndirectPeer;
    }

    merged.last_successful_address = local.last_successful_address.or(incoming.last_successful_address);
    let incoming_first = if incoming.first_seen_unix_ms == 0 {
        now_ms
    } else {
        incoming.first_seen_unix_ms
    };
    merged.first_seen_unix_ms = local.first_seen_unix_ms.min(incoming_first);
    merged.record_updated_at_unix_ms = now_ms
        .max(local.record_updated_at_unix_ms)
        .max(incoming.record_updated_at_unix_ms);
    Ok(merged)
}

fn pick_candidate(
    left: &EndpointCandidate,
    right: &EndpointCandidate,
    allow_remote_reachability_upgrade: bool,
) -> EndpointCandidate {
    let newer = match right.observed_at_unix_ms.cmp(&left.observed_at_unix_ms) {
        Ordering::Greater => right,
        Ordering::Less => left,
        Ordering::Equal => {
            if right.kind.priority() != left.kind.priority() {
                if right.kind.priority() > left.kind.priority() {
                    right
                } else {
                    left
                }
            } else if right.priority != left.priority {
                if right.priority > left.priority {
                    right
                } else {
                    left
                }
            } else if right.address.to_string() > left.address.to_string() {
                right
            } else {
                left
            }
        }
    };

    let older = if std::ptr::eq(newer, left) {
        right
    } else {
        left
    };
    let mut selected = newer.clone();
    selected.reachability = preserve_reachability(
        left.reachability,
        right.reachability,
        allow_remote_reachability_upgrade,
    );

    if selected.expires_at_unix_ms.is_none() {
        selected.expires_at_unix_ms = older.expires_at_unix_ms.or(newer.expires_at_unix_ms);
    }
    if selected.source_node_id.is_none() {
        selected.source_node_id = older.source_node_id.or(newer.source_node_id);
    }
    selected
}

fn preserve_reachability(
    left: CandidateReachability,
    right: CandidateReachability,
    allow_remote_reachability_upgrade: bool,
) -> CandidateReachability {
    if left == CandidateReachability::Reachable {
        return CandidateReachability::Reachable;
    }
    if right == CandidateReachability::Reachable {
        if allow_remote_reachability_upgrade {
            return CandidateReachability::Reachable;
        }
        if left == CandidateReachability::Unreachable {
            return CandidateReachability::Unreachable;
        }
        return left;
    }
    if left == CandidateReachability::Unreachable {
        return CandidateReachability::Unreachable;
    }
    if right == CandidateReachability::Unreachable {
        if allow_remote_reachability_upgrade {
            return CandidateReachability::Unreachable;
        }
        return left;
    }
    right
}

fn trim_candidates(candidates: &mut Vec<EndpointCandidate>, now_ms: i64) {
    if candidates.len() <= MAX_PEER_CANDIDATES {
        return;
    }
    candidates.sort_by(|left, right| {
        let left_expired = left.is_expired(now_ms);
        let right_expired = right.is_expired(now_ms);
        left_expired
            .cmp(&right_expired)
            .then_with(|| {
                let left_unreach = left.reachability == CandidateReachability::Unreachable;
                let right_unreach = right.reachability == CandidateReachability::Unreachable;
                left_unreach.cmp(&right_unreach)
            })
            .then_with(|| left.observed_at_unix_ms.cmp(&right.observed_at_unix_ms))
            .then_with(|| left.kind.priority().cmp(&right.kind.priority()))
    });
    while candidates.len() > MAX_PEER_CANDIDATES {
        candidates.remove(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cert_and_id(byte: u8) -> (Vec<u8>, NodeId) {
        let certificate_der = vec![byte; 32];
        let node_id = NodeId::from_certificate_der(&certificate_der);
        (certificate_der, node_id)
    }

    #[test]
    fn merges_candidates_by_address_and_prefers_newer() {
        let now = 1_000_000;
        let older = EndpointCandidate::new_at(
            CandidateKind::LocalNetwork,
            "192.168.1.10:7000".parse().unwrap(),
            now - 10_000,
        );
        let newer = EndpointCandidate::new_at(
            CandidateKind::PeerObserved,
            "192.168.1.10:7000".parse().unwrap(),
            now - 1_000,
        );
        let merged = merge_candidates(&[older], &[newer.clone()], now, false);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].observed_at_unix_ms, newer.observed_at_unix_ms);
        assert_eq!(merged[0].kind, CandidateKind::PeerObserved);
    }

    #[test]
    fn remote_gossip_does_not_upgrade_unreachable() {
        let now = 1_000_000;
        let local = EndpointCandidate::new_at(
            CandidateKind::PublicIpv4,
            "203.0.113.8:7000".parse().unwrap(),
            now - 1_000,
        )
        .with_reachability(CandidateReachability::Unreachable);
        let remote = EndpointCandidate::new_at(
            CandidateKind::PublicIpv4,
            "203.0.113.8:7000".parse().unwrap(),
            now,
        )
        .with_reachability(CandidateReachability::Reachable);
        let merged = merge_candidates(&[local], &[remote], now, false);
        assert_eq!(merged[0].reachability, CandidateReachability::Unreachable);
    }

    #[test]
    fn drops_expired_candidates_from_advertisement() {
        let now = 10_000;
        let expired = EndpointCandidate::new_at(
            CandidateKind::PeerObserved,
            "198.51.100.2:9".parse().unwrap(),
            0,
        )
        .with_expiry(Some(1));
        let live = EndpointCandidate::new_at(
            CandidateKind::GlobalIpv6,
            "[2001:db8::1]:7000".parse().unwrap(),
            now,
        );
        let advertised = filter_advertised_candidates(&[expired, live.clone()], now);
        assert_eq!(advertised, vec![live]);
    }

    #[test]
    fn merge_preserves_local_successful_address() {
        let now = 50_000;
        let (certificate_der, id) = cert_and_id(7);
        let local = PeerRecord {
            node_id: id,
            display_name: "alpha".into(),
            certificate_der: certificate_der.clone(),
            candidates: vec![],
            last_successful_address: Some("10.0.0.2:7000".parse().unwrap()),
            last_seen_unix_ms: Some(40_000),
            first_seen_unix_ms: 10_000,
            record_updated_at_unix_ms: 40_000,
            origin: PeerRecordOrigin::DirectPeer,
        };
        let incoming = PeerRecord {
            node_id: id,
            display_name: "alpha-remote".into(),
            certificate_der,
            candidates: vec![EndpointCandidate::new_at(
                CandidateKind::LocalNetwork,
                "10.0.0.3:7000".parse().unwrap(),
                now,
            )],
            last_successful_address: Some("10.0.0.9:7000".parse().unwrap()),
            last_seen_unix_ms: Some(12_000),
            first_seen_unix_ms: 12_000,
            record_updated_at_unix_ms: now,
            origin: PeerRecordOrigin::IndirectPeer,
        };
        let merged = merge_peer_records(
            Some(&local),
            &incoming,
            NodeId::from_bytes([1; 32]),
            now,
            false,
        )
        .expect("merge");
        assert_eq!(
            merged.last_successful_address,
            Some("10.0.0.2:7000".parse().unwrap())
        );
        assert_eq!(merged.last_seen_unix_ms, Some(40_000));
        assert_eq!(merged.display_name, "alpha-remote");
        assert_eq!(merged.candidates.len(), 1);
    }
}
