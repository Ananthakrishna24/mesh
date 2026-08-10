use rusqlite::{Connection, params};

use crate::StoreResult;

pub fn set_step(conn: &Connection, step: &str) -> StoreResult<()> {
    conn.execute(
        r#"
        INSERT INTO onboarding (id, last_step) VALUES (1, ?1)
        ON CONFLICT(id) DO UPDATE SET last_step = excluded.last_step
        "#,
        params![step],
    )?;
    Ok(())
}
