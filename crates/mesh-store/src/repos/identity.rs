use mesh_core::{LocalIdentity, MeshId, NodeId, identity_matches};
use rusqlite::{Connection, OptionalExtension, params};

use crate::{StoreError, StoreResult};

pub fn load(conn: &Connection) -> StoreResult<Option<LocalIdentity>> {
    let row = conn
        .query_row(
            r#"
            SELECT node_id, mesh_id, display_name, certificate_der, private_key_der, created_at_unix_ms
            FROM local_identity
            WHERE id = 1
            "#,
            [],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            },
        )
        .optional()?;

    let Some((node_id, mesh_id, display_name, certificate_der, private_key_der, created_at_unix_ms)) =
        row
    else {
        return Ok(None);
    };

    let identity = LocalIdentity {
        node_id: NodeId::from_slice(&node_id)?,
        mesh_id: MeshId::from_slice(&mesh_id)?,
        display_name,
        certificate_der,
        private_key_der,
        created_at_unix_ms,
    };

    if !identity_matches(&identity) {
        return Err(StoreError::Corrupt(
            "stored identity does not match certificate-derived node id".to_owned(),
        ));
    }

    Ok(Some(identity))
}

pub fn insert(conn: &Connection, identity: &LocalIdentity) -> StoreResult<()> {
    conn.execute(
        r#"
        INSERT INTO local_identity (
            id, node_id, mesh_id, display_name, certificate_der, private_key_der, created_at_unix_ms
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
        params![
            1i32,
            identity.node_id.as_bytes().as_slice(),
            identity.mesh_id.as_bytes().as_slice(),
            identity.display_name,
            identity.certificate_der,
            identity.private_key_der,
            identity.created_at_unix_ms,
        ],
    )?;
    Ok(())
}
