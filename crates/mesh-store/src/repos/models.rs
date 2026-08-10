use mesh_core::{CacheValidationState, ModelCacheEntry, ModelFormat, ModelManifestRecord};
use rusqlite::{Connection, OptionalExtension, params};

use crate::{StoreError, StoreResult};

pub fn upsert_manifest(conn: &Connection, record: &ModelManifestRecord) -> StoreResult<()> {
    conn.execute(
        r#"
        INSERT INTO model_manifests (
            cache_key, provider, repository, revision, adapter_id, adapter_version,
            model_format, quantization, manifest_hash, canonical_bytes, created_at_unix_ms
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        ON CONFLICT(cache_key) DO UPDATE SET
            provider = excluded.provider,
            repository = excluded.repository,
            revision = excluded.revision,
            adapter_id = excluded.adapter_id,
            adapter_version = excluded.adapter_version,
            model_format = excluded.model_format,
            quantization = excluded.quantization,
            manifest_hash = excluded.manifest_hash,
            canonical_bytes = excluded.canonical_bytes,
            created_at_unix_ms = excluded.created_at_unix_ms
        "#,
        params![
            record.cache_key,
            record.provider,
            record.repository,
            record.revision,
            record.adapter_id,
            record.adapter_version,
            record.model_format.as_str(),
            record.quantization,
            record.manifest_hash,
            record.canonical_bytes,
            record.created_at_unix_ms,
        ],
    )?;
    Ok(())
}

pub fn get_manifest(
    conn: &Connection,
    cache_key: &str,
) -> StoreResult<Option<ModelManifestRecord>> {
    conn.query_row(
        r#"
        SELECT cache_key, provider, repository, revision, adapter_id, adapter_version,
               model_format, quantization, manifest_hash, canonical_bytes, created_at_unix_ms
        FROM model_manifests
        WHERE cache_key = ?1
        "#,
        params![cache_key],
        |row| {
            Ok(ModelManifestRecord {
                cache_key: row.get(0)?,
                provider: row.get(1)?,
                repository: row.get(2)?,
                revision: row.get(3)?,
                adapter_id: row.get(4)?,
                adapter_version: row.get(5)?,
                model_format: ModelFormat::parse(&row.get::<_, String>(6)?)
                    .ok_or_else(|| rusqlite::Error::InvalidQuery)?,
                quantization: row.get(7)?,
                manifest_hash: row.get(8)?,
                canonical_bytes: row.get(9)?,
                created_at_unix_ms: row.get(10)?,
            })
        },
    )
    .optional()
    .map_err(StoreError::from)
}

pub fn upsert_cache_entry(conn: &Connection, entry: &ModelCacheEntry) -> StoreResult<()> {
    conn.execute(
        r#"
        INSERT INTO model_cache_entries (
            entry_id, provider, repository, revision, artifact_path, relative_path,
            byte_length, range_start, range_end, etag, digest_hex, dtype, shape_json,
            state, reference_count, pinned, last_used_at_unix_ms, created_at_unix_ms
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18
        )
        ON CONFLICT(entry_id) DO UPDATE SET
            provider = excluded.provider,
            repository = excluded.repository,
            revision = excluded.revision,
            artifact_path = excluded.artifact_path,
            relative_path = excluded.relative_path,
            byte_length = excluded.byte_length,
            range_start = excluded.range_start,
            range_end = excluded.range_end,
            etag = excluded.etag,
            digest_hex = excluded.digest_hex,
            dtype = excluded.dtype,
            shape_json = excluded.shape_json,
            state = excluded.state,
            reference_count = excluded.reference_count,
            pinned = excluded.pinned,
            last_used_at_unix_ms = excluded.last_used_at_unix_ms,
            created_at_unix_ms = excluded.created_at_unix_ms
        "#,
        params![
            entry.entry_id,
            entry.provider,
            entry.repository,
            entry.revision,
            entry.artifact_path,
            entry.relative_path,
            entry.byte_length as i64,
            entry.range_start.map(|value| value as i64),
            entry.range_end.map(|value| value as i64),
            entry.etag,
            entry.digest_hex,
            entry.dtype,
            entry.shape_json,
            entry.state.as_str(),
            entry.reference_count as i64,
            entry.pinned as i64,
            entry.last_used_at_unix_ms,
            entry.created_at_unix_ms,
        ],
    )?;
    Ok(())
}

pub fn get_cache_entry(conn: &Connection, entry_id: &str) -> StoreResult<Option<ModelCacheEntry>> {
    conn.query_row(
        r#"
        SELECT entry_id, provider, repository, revision, artifact_path, relative_path,
               byte_length, range_start, range_end, etag, digest_hex, dtype, shape_json,
               state, reference_count, pinned, last_used_at_unix_ms, created_at_unix_ms
        FROM model_cache_entries
        WHERE entry_id = ?1
        "#,
        params![entry_id],
        decode_cache_entry,
    )
    .optional()
    .map_err(StoreError::from)
}

pub fn list_cache_entries(conn: &Connection) -> StoreResult<Vec<ModelCacheEntry>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT entry_id, provider, repository, revision, artifact_path, relative_path,
               byte_length, range_start, range_end, etag, digest_hex, dtype, shape_json,
               state, reference_count, pinned, last_used_at_unix_ms, created_at_unix_ms
        FROM model_cache_entries
        ORDER BY last_used_at_unix_ms ASC
        "#,
    )?;
    let rows = stmt.query_map([], decode_cache_entry)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn delete_cache_entry(conn: &Connection, entry_id: &str) -> StoreResult<()> {
    conn.execute(
        "DELETE FROM model_cache_entries WHERE entry_id = ?1",
        params![entry_id],
    )?;
    Ok(())
}

pub fn cache_usage_bytes(conn: &Connection) -> StoreResult<(u64, u64, u32, u32)> {
    let (used, protected, entry_count, partial_count) = conn.query_row(
        r#"
        SELECT
            COALESCE(SUM(byte_length), 0),
            COALESCE(SUM(CASE
                WHEN reference_count > 0 OR pinned = 1 THEN byte_length
                ELSE 0
            END), 0),
            COUNT(*),
            COALESCE(SUM(CASE WHEN state = 'partial' THEN 1 ELSE 0 END), 0)
        FROM model_cache_entries
        "#,
        [],
        |row| {
            Ok((
                row.get::<_, i64>(0)? as u64,
                row.get::<_, i64>(1)? as u64,
                row.get::<_, i64>(2)? as u32,
                row.get::<_, i64>(3)? as u32,
            ))
        },
    )?;
    Ok((used, protected, entry_count, partial_count))
}

fn decode_cache_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<ModelCacheEntry> {
    let state = CacheValidationState::parse(&row.get::<_, String>(13)?)
        .ok_or(rusqlite::Error::InvalidQuery)?;
    Ok(ModelCacheEntry {
        entry_id: row.get(0)?,
        provider: row.get(1)?,
        repository: row.get(2)?,
        revision: row.get(3)?,
        artifact_path: row.get(4)?,
        relative_path: row.get(5)?,
        byte_length: row.get::<_, i64>(6)? as u64,
        range_start: row.get::<_, Option<i64>>(7)?.map(|value| value as u64),
        range_end: row.get::<_, Option<i64>>(8)?.map(|value| value as u64),
        etag: row.get(9)?,
        digest_hex: row.get(10)?,
        dtype: row.get(11)?,
        shape_json: row.get(12)?,
        state,
        reference_count: row.get::<_, i64>(14)? as u32,
        pinned: row.get::<_, i64>(15)? != 0,
        last_used_at_unix_ms: row.get(16)?,
        created_at_unix_ms: row.get(17)?,
    })
}
