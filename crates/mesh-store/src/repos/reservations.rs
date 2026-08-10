use mesh_core::{
    DeploymentId, GpuResourceAmount, LocalReservation, NodeId, ReservationId, ReservationState,
    ResourceAmount,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::{StoreError, StoreResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AmountJson {
    system_memory_bytes: u64,
    disk_bytes: u64,
    execution_slots: u32,
    gpus: Vec<GpuResourceAmount>,
}

impl From<&ResourceAmount> for AmountJson {
    fn from(value: &ResourceAmount) -> Self {
        Self {
            system_memory_bytes: value.system_memory_bytes,
            disk_bytes: value.disk_bytes,
            execution_slots: value.execution_slots,
            gpus: value.gpus.clone(),
        }
    }
}

impl From<AmountJson> for ResourceAmount {
    fn from(value: AmountJson) -> Self {
        Self {
            system_memory_bytes: value.system_memory_bytes,
            disk_bytes: value.disk_bytes,
            execution_slots: value.execution_slots,
            gpus: value.gpus,
        }
    }
}

pub fn list_active(conn: &Connection, now_unix_ms: i64) -> StoreResult<Vec<LocalReservation>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT
            reservation_id,
            deployment_id,
            owner_node_id,
            amount_json,
            state,
            purpose,
            expires_at_unix_ms,
            created_at_unix_ms,
            updated_at_unix_ms
        FROM reservations
        WHERE expires_at_unix_ms > ?1
        ORDER BY created_at_unix_ms ASC
        "#,
    )?;
    let rows = stmt.query_map(params![now_unix_ms], |row| {
        Ok((
            row.get::<_, Vec<u8>>(0)?,
            row.get::<_, Vec<u8>>(1)?,
            row.get::<_, Vec<u8>>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, i64>(6)?,
            row.get::<_, i64>(7)?,
            row.get::<_, i64>(8)?,
        ))
    })?;

    let mut items = Vec::new();
    for row in rows {
        let (
            reservation_id,
            deployment_id,
            owner_node_id,
            amount_json,
            state,
            purpose,
            expires_at_unix_ms,
            created_at_unix_ms,
            updated_at_unix_ms,
        ) = row?;
        items.push(decode_reservation(
            reservation_id,
            deployment_id,
            owner_node_id,
            amount_json,
            state,
            purpose,
            expires_at_unix_ms,
            created_at_unix_ms,
            updated_at_unix_ms,
        )?);
    }
    Ok(items)
}

pub fn upsert(conn: &Connection, reservation: &LocalReservation) -> StoreResult<()> {
    let amount_json = serde_json::to_string(&AmountJson::from(&reservation.amount))
        .map_err(|error| StoreError::Corrupt(format!("encode reservation amount: {error}")))?;
    conn.execute(
        r#"
        INSERT INTO reservations (
            reservation_id,
            deployment_id,
            owner_node_id,
            amount_json,
            state,
            purpose,
            expires_at_unix_ms,
            created_at_unix_ms,
            updated_at_unix_ms
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        ON CONFLICT(reservation_id) DO UPDATE SET
            deployment_id = excluded.deployment_id,
            owner_node_id = excluded.owner_node_id,
            amount_json = excluded.amount_json,
            state = excluded.state,
            purpose = excluded.purpose,
            expires_at_unix_ms = excluded.expires_at_unix_ms,
            updated_at_unix_ms = excluded.updated_at_unix_ms
        "#,
        params![
            reservation.reservation_id.as_bytes().as_slice(),
            reservation.deployment_id.as_bytes().as_slice(),
            reservation.owner_node_id.as_bytes().as_slice(),
            amount_json,
            reservation.state.as_str(),
            reservation.purpose,
            reservation.expires_at_unix_ms,
            reservation.created_at_unix_ms,
            reservation.updated_at_unix_ms,
        ],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, reservation_id: ReservationId) -> StoreResult<()> {
    conn.execute(
        "DELETE FROM reservations WHERE reservation_id = ?1",
        params![reservation_id.as_bytes().as_slice()],
    )?;
    Ok(())
}

pub fn delete_owner(conn: &Connection, owner_node_id: NodeId) -> StoreResult<usize> {
    let changed = conn.execute(
        "DELETE FROM reservations WHERE owner_node_id = ?1",
        params![owner_node_id.as_bytes().as_slice()],
    )?;
    Ok(changed)
}

pub fn delete_all(conn: &Connection) -> StoreResult<usize> {
    let changed = conn.execute("DELETE FROM reservations", [])?;
    Ok(changed)
}

pub fn delete_expired(conn: &Connection, now_unix_ms: i64) -> StoreResult<usize> {
    let changed = conn.execute(
        "DELETE FROM reservations WHERE expires_at_unix_ms <= ?1",
        params![now_unix_ms],
    )?;
    Ok(changed)
}

pub fn get(
    conn: &Connection,
    reservation_id: ReservationId,
) -> StoreResult<Option<LocalReservation>> {
    conn.query_row(
        r#"
        SELECT
            reservation_id,
            deployment_id,
            owner_node_id,
            amount_json,
            state,
            purpose,
            expires_at_unix_ms,
            created_at_unix_ms,
            updated_at_unix_ms
        FROM reservations
        WHERE reservation_id = ?1
        "#,
        params![reservation_id.as_bytes().as_slice()],
        |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, i64>(8)?,
            ))
        },
    )
    .optional()?
    .map(
        |(
            reservation_id,
            deployment_id,
            owner_node_id,
            amount_json,
            state,
            purpose,
            expires_at_unix_ms,
            created_at_unix_ms,
            updated_at_unix_ms,
        )| {
            decode_reservation(
                reservation_id,
                deployment_id,
                owner_node_id,
                amount_json,
                state,
                purpose,
                expires_at_unix_ms,
                created_at_unix_ms,
                updated_at_unix_ms,
            )
        },
    )
    .transpose()
}

fn decode_reservation(
    reservation_id: Vec<u8>,
    deployment_id: Vec<u8>,
    owner_node_id: Vec<u8>,
    amount_json: String,
    state: String,
    purpose: String,
    expires_at_unix_ms: i64,
    created_at_unix_ms: i64,
    updated_at_unix_ms: i64,
) -> StoreResult<LocalReservation> {
    let amount: AmountJson = serde_json::from_str(&amount_json)
        .map_err(|error| StoreError::Corrupt(format!("decode reservation amount: {error}")))?;
    let state = ReservationState::parse(&state)
        .ok_or_else(|| StoreError::Corrupt(format!("unknown reservation state {state}")))?;
    Ok(LocalReservation {
        reservation_id: ReservationId::from_slice(&reservation_id)?,
        deployment_id: DeploymentId::from_slice(&deployment_id)?,
        owner_node_id: NodeId::from_slice(&owner_node_id)?,
        amount: amount.into(),
        state,
        purpose,
        expires_at_unix_ms,
        created_at_unix_ms,
        updated_at_unix_ms,
    })
}
