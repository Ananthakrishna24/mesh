use mesh_core::{CandidateKind, EndpointCandidate, NodeId, PeerRecord};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::{StoreError, StoreResult};

pub fn list(conn: &Connection) -> StoreResult<Vec<PeerRecord>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT node_id, display_name, certificate_der, candidates_json
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
        ))
    })?;

    let mut peers = Vec::new();
    for row in rows {
        let (node_id, display_name, certificate_der, candidates_json) = row?;
        let candidates = decode_candidates(&candidates_json)?;
        peers.push(PeerRecord {
            node_id: NodeId::from_slice(&node_id)?,
            display_name,
            certificate_der,
            candidates,
        });
    }
    Ok(peers)
}

pub fn upsert(conn: &Connection, peer: &PeerRecord) -> StoreResult<()> {
    let candidates_json = encode_candidates(&peer.candidates)?;
    conn.execute(
        r#"
        INSERT INTO peers (node_id, display_name, certificate_der, candidates_json)
        VALUES (?1, ?2, ?3, ?4)
        ON CONFLICT(node_id) DO UPDATE SET
            display_name = excluded.display_name,
            certificate_der = excluded.certificate_der,
            candidates_json = excluded.candidates_json
        "#,
        params![
            peer.node_id.as_bytes().as_slice(),
            peer.display_name,
            peer.certificate_der,
            candidates_json,
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
}

fn encode_candidates(candidates: &[EndpointCandidate]) -> StoreResult<String> {
    let stored = candidates
        .iter()
        .map(|candidate| StoredCandidate {
            kind: kind_name(candidate.kind).to_owned(),
            address: candidate.address.to_string(),
            priority: candidate.priority,
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
            Ok(EndpointCandidate {
                kind,
                address,
                priority: candidate.priority,
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
