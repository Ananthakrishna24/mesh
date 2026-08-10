use mesh_core::{EnrollmentId, InvitationRecord, InvitationState, NodeId};
use rusqlite::{Connection, OptionalExtension, params};

use crate::{StoreError, StoreResult};

pub fn insert(conn: &Connection, record: &InvitationRecord) -> StoreResult<()> {
    conn.execute(
        r#"
        INSERT INTO invitations (
            enrollment_id, secret_digest, expires_at_unix_ms, state, bound_node_id, created_at_unix_ms
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
        params![
            record.enrollment_id.as_bytes().as_slice(),
            record.secret_digest.as_slice(),
            record.expires_at_unix_ms,
            state_name(record.state),
            record
                .bound_node_id
                .map(|node_id| node_id.as_bytes().to_vec()),
            record.created_at_unix_ms,
        ],
    )?;
    Ok(())
}

pub fn get(conn: &Connection, enrollment_id: EnrollmentId) -> StoreResult<Option<InvitationRecord>> {
    conn.query_row(
        r#"
        SELECT enrollment_id, secret_digest, expires_at_unix_ms, state, bound_node_id, created_at_unix_ms
        FROM invitations
        WHERE enrollment_id = ?1
        "#,
        params![enrollment_id.as_bytes().as_slice()],
        |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<Vec<u8>>>(4)?,
                row.get::<_, i64>(5)?,
            ))
        },
    )
    .optional()?
    .map(|(enrollment_id, secret_digest, expires_at_unix_ms, state, bound_node_id, created_at_unix_ms)| {
        let digest: [u8; 32] = secret_digest
            .as_slice()
            .try_into()
            .map_err(|_| StoreError::Corrupt("invitation secret digest must be 32 bytes".to_owned()))?;
        Ok(InvitationRecord {
            enrollment_id: EnrollmentId::from_slice(&enrollment_id)?,
            secret_digest: digest,
            expires_at_unix_ms,
            state: parse_state(&state)?,
            bound_node_id: bound_node_id
                .map(|bytes| NodeId::from_slice(&bytes))
                .transpose()?,
            created_at_unix_ms,
        })
    })
    .transpose()
}

pub fn set_state(
    conn: &Connection,
    enrollment_id: EnrollmentId,
    state: InvitationState,
    bound_node_id: Option<NodeId>,
) -> StoreResult<()> {
    let changed = conn.execute(
        r#"
        UPDATE invitations
        SET state = ?1, bound_node_id = ?2
        WHERE enrollment_id = ?3
        "#,
        params![
            state_name(state),
            bound_node_id.map(|node_id| node_id.as_bytes().to_vec()),
            enrollment_id.as_bytes().as_slice(),
        ],
    )?;
    if changed != 1 {
        return Err(StoreError::NotFound("invitation not found".to_owned()));
    }
    Ok(())
}

fn state_name(state: InvitationState) -> &'static str {
    match state {
        InvitationState::Pending => "pending",
        InvitationState::Bound => "bound",
        InvitationState::Consumed => "consumed",
        InvitationState::Expired => "expired",
    }
}

fn parse_state(value: &str) -> StoreResult<InvitationState> {
    match value {
        "pending" => Ok(InvitationState::Pending),
        "bound" => Ok(InvitationState::Bound),
        "consumed" => Ok(InvitationState::Consumed),
        "expired" => Ok(InvitationState::Expired),
        other => Err(StoreError::Corrupt(format!(
            "unknown invitation state {other}"
        ))),
    }
}
