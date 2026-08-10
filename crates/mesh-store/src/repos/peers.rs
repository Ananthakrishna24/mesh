use mesh_core::{
    CandidateKind, CandidateReachability, EndpointCandidate, NodeId, PeerRecord, PeerRecordOrigin,
};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::{StoreError, StoreResult};

pub fn list(conn: &Connection) -> StoreResult<Vec<PeerRecord>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT
            node_id,
            display_name,
            certificate_der,
            candidates_json,
            last_successful_address,
            last_seen_unix_ms,
            first_seen_unix_ms,
            record_updated_at_unix_ms,
            origin
        FROM peers
        ORDER BY display_name COLLATE NOCASE ASC
        "#,
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, Vec<u8>>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Vec<u8>>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<i64>>(5)?,
            row.get::<_, i64>(6)?,
            row.get::<_, i64>(7)?,
            row.get::<_, String>(8)?,
        ))
    })?;

    let mut peers = Vec::new();
    for row in rows {
        let (
            node_id,
            display_name,
            certificate_der,
            candidates_json,
            last_successful_address,
            last_seen_unix_ms,
            first_seen_unix_ms,
            record_updated_at_unix_ms,
            origin,
        ) = row?;
        let candidates = decode_candidates(&candidates_json)?;
        peers.push(PeerRecord {
            node_id: NodeId::from_slice(&node_id)?,
            display_name,
            certificate_der,
            candidates,
            last_successful_address: last_successful_address
                .map(|value| {
                    value
                        .parse()
                        .map_err(|error| StoreError::Corrupt(format!("invalid address: {error}")))
                })
                .transpose()?,
            last_seen_unix_ms,
            first_seen_unix_ms,
            record_updated_at_unix_ms,
            origin: decode_origin(&origin)?,
        });
    }
    Ok(peers)
}

pub fn upsert(conn: &Connection, peer: &PeerRecord) -> StoreResult<()> {
    let candidates_json = encode_candidates(&peer.candidates)?;
    conn.execute(
        r#"
        INSERT INTO peers (
            node_id,
            display_name,
            certificate_der,
            candidates_json,
            last_successful_address,
            last_seen_unix_ms,
            first_seen_unix_ms,
            record_updated_at_unix_ms,
            origin
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        ON CONFLICT(node_id) DO UPDATE SET
            display_name = excluded.display_name,
            certificate_der = excluded.certificate_der,
            candidates_json = excluded.candidates_json,
            last_successful_address = excluded.last_successful_address,
            last_seen_unix_ms = excluded.last_seen_unix_ms,
            first_seen_unix_ms = excluded.first_seen_unix_ms,
            record_updated_at_unix_ms = excluded.record_updated_at_unix_ms,
            origin = excluded.origin
        "#,
        params![
            peer.node_id.as_bytes().as_slice(),
            peer.display_name,
            peer.certificate_der,
            candidates_json,
            peer.last_successful_address
                .map(|address| address.to_string()),
            peer.last_seen_unix_ms,
            peer.first_seen_unix_ms,
            peer.record_updated_at_unix_ms,
            origin_name(peer.origin),
        ],
    )?;
    Ok(())
}

pub fn clear(conn: &Connection) -> StoreResult<()> {
    conn.execute("DELETE FROM peers", [])?;
    Ok(())
}


#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredCandidate {
    kind: String,
    address: String,
    priority: u16,
    #[serde(default)]
    observed_at_unix_ms: Option<i64>,
    #[serde(default)]
    expires_at_unix_ms: Option<i64>,
    #[serde(default)]
    source_node_id: Option<String>,
    #[serde(default)]
    reachability: Option<String>,
}

fn encode_candidates(candidates: &[EndpointCandidate]) -> StoreResult<String> {
    let stored = candidates
        .iter()
        .map(|candidate| StoredCandidate {
            kind: kind_name(candidate.kind).to_owned(),
            address: candidate.address.to_string(),
            priority: candidate.priority,
            observed_at_unix_ms: Some(candidate.observed_at_unix_ms),
            expires_at_unix_ms: candidate.expires_at_unix_ms,
            source_node_id: candidate
                .source_node_id
                .map(|node_id| hex::encode(node_id.as_bytes())),
            reachability: Some(reachability_name(candidate.reachability).to_owned()),
        })
        .collect::<Vec<_>>();
    serde_json::to_string(&stored).map_err(|error| StoreError::Corrupt(error.to_string()))
}

fn decode_candidates(value: &str) -> StoreResult<Vec<EndpointCandidate>> {
    let stored: Vec<StoredCandidate> =
        serde_json::from_str(value).map_err(|error| StoreError::Corrupt(error.to_string()))?;
    stored
        .into_iter()
        .map(|candidate| {
            let kind = match candidate.kind.as_str() {
                "global_ipv6" => CandidateKind::GlobalIpv6,
                "public_ipv4" => CandidateKind::PublicIpv4,
                "router_mapping" => CandidateKind::RouterMapping,
                "manual" => CandidateKind::Manual,
                "peer_observed" => CandidateKind::PeerObserved,
                "local_network" => CandidateKind::LocalNetwork,
                other => {
                    return Err(StoreError::Corrupt(format!(
                        "unknown candidate kind {other}"
                    )));
                }
            };
            let address = candidate
                .address
                .parse()
                .map_err(|error| StoreError::Corrupt(format!("invalid candidate address: {error}")))?;
            let observed_at_unix_ms = candidate.observed_at_unix_ms.unwrap_or(0);
            let expires_at_unix_ms = candidate
                .expires_at_unix_ms
                .or_else(|| kind.default_expiry(observed_at_unix_ms));
            let source_node_id = candidate
                .source_node_id
                .map(|value| NodeId::parse_hex(&value))
                .transpose()
                .map_err(|error| StoreError::Corrupt(error.to_string()))?;
            let reachability = match candidate.reachability.as_deref() {
                Some("reachable") => CandidateReachability::Reachable,
                Some("unreachable") => CandidateReachability::Unreachable,
                Some("unknown") | None => CandidateReachability::Unknown,
                Some(other) => {
                    return Err(StoreError::Corrupt(format!(
                        "unknown reachability {other}"
                    )));
                }
            };
            Ok(EndpointCandidate {
                kind,
                address,
                priority: candidate.priority,
                observed_at_unix_ms,
                expires_at_unix_ms,
                source_node_id,
                reachability,
            })
        })
        .collect()
}

fn kind_name(kind: CandidateKind) -> &'static str {
    match kind {
        CandidateKind::GlobalIpv6 => "global_ipv6",
        CandidateKind::PublicIpv4 => "public_ipv4",
        CandidateKind::RouterMapping => "router_mapping",
        CandidateKind::Manual => "manual",
        CandidateKind::PeerObserved => "peer_observed",
        CandidateKind::LocalNetwork => "local_network",
    }
}

fn reachability_name(value: CandidateReachability) -> &'static str {
    match value {
        CandidateReachability::Unknown => "unknown",
        CandidateReachability::Reachable => "reachable",
        CandidateReachability::Unreachable => "unreachable",
    }
}

fn origin_name(origin: PeerRecordOrigin) -> &'static str {
    match origin {
        PeerRecordOrigin::LocalSelf => "local_self",
        PeerRecordOrigin::DirectPeer => "direct_peer",
        PeerRecordOrigin::IndirectPeer => "indirect_peer",
    }
}

fn decode_origin(value: &str) -> StoreResult<PeerRecordOrigin> {
    match value {
        "local_self" => Ok(PeerRecordOrigin::LocalSelf),
        "direct_peer" => Ok(PeerRecordOrigin::DirectPeer),
        "indirect_peer" => Ok(PeerRecordOrigin::IndirectPeer),
        other => Err(StoreError::Corrupt(format!("unknown peer origin {other}"))),
    }
}
