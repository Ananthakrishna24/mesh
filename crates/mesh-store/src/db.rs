use std::time::{SystemTime, UNIX_EPOCH};

use mesh_core::{
    EnrollmentId, InvitationRecord, InvitationState, LocalIdentity, LocalReservation, MeshId,
    ModelCacheEntry, ModelCacheView, ModelManifestRecord, NodeId, PeerRecord, ReservationId,
    identity_matches,
};


use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};

use crate::paths::StorePaths;
use crate::repos;
use crate::{StoreError, StoreResult};

pub const SCHEMA_VERSION: i32 = 4;


#[derive(Debug)]
pub struct Store {
    conn: Connection,
    paths: StorePaths,
}

impl Store {
    pub fn open(paths: StorePaths) -> StoreResult<Self> {
        std::fs::create_dir_all(&paths.data_dir)?;
        std::fs::create_dir_all(&paths.cache_dir)?;
        std::fs::create_dir_all(&paths.model_cache_dir)?;
        let conn = Connection::open(&paths.db_path)?;
        let store = Self { conn, paths };
        store.configure()?;
        store.migrate()?;
        Ok(store)
    }

    pub fn paths(&self) -> &StorePaths {
        &self.paths
    }

    pub fn load_identity(&self) -> StoreResult<Option<LocalIdentity>> {
        repos::identity::load(&self.conn)
    }

    pub fn create_mesh_identity(
        &mut self,
        display_name: String,
        certificate_der: Vec<u8>,
        private_key_der: Vec<u8>,
    ) -> StoreResult<LocalIdentity> {
        let node_id = NodeId::from_certificate_der(&certificate_der);
        let mesh_id = MeshId::new();
        let created_at_unix_ms = now_unix_ms();
        let identity = LocalIdentity {
            node_id,
            mesh_id,
            display_name: display_name.trim().to_owned(),
            certificate_der,
            private_key_der,
            created_at_unix_ms,
        };
        if !identity_matches(&identity) {
            return Err(StoreError::Corrupt(
                "generated identity failed validation".to_owned(),
            ));
        }

        let tx = self.conn.transaction()?;
        if repos::identity::load(&tx)?.is_some() {
            return Err(StoreError::Corrupt(
                "local identity already exists".to_owned(),
            ));
        }
        repos::identity::insert(&tx, &identity)?;
        repos::onboarding::set_step(&tx, "enrolled")?;
        tx.commit()?;
        Ok(identity)
    }

    pub fn create_joining_identity(
        &mut self,
        display_name: String,
        mesh_id: MeshId,
        certificate_der: Vec<u8>,
        private_key_der: Vec<u8>,
    ) -> StoreResult<LocalIdentity> {
        let node_id = NodeId::from_certificate_der(&certificate_der);
        let created_at_unix_ms = now_unix_ms();
        let identity = LocalIdentity {
            node_id,
            mesh_id,
            display_name: display_name.trim().to_owned(),
            certificate_der,
            private_key_der,
            created_at_unix_ms,
        };
        if !identity_matches(&identity) {
            return Err(StoreError::Corrupt(
                "generated identity failed validation".to_owned(),
            ));
        }

        let tx = self.conn.transaction()?;
        if repos::identity::load(&tx)?.is_some() {
            return Err(StoreError::Corrupt(
                "local identity already exists".to_owned(),
            ));
        }
        repos::identity::insert(&tx, &identity)?;
        repos::onboarding::set_step(&tx, "joining")?;
        tx.commit()?;
        Ok(identity)
    }

    pub fn mark_enrolled(&mut self) -> StoreResult<()> {
        let tx = self.conn.transaction()?;
        repos::onboarding::set_step(&tx, "enrolled")?;
        tx.commit()?;
        Ok(())
    }

    pub fn list_peers(&self) -> StoreResult<Vec<PeerRecord>> {
        repos::peers::list(&self.conn)
    }

    pub fn upsert_peer(&mut self, peer: &PeerRecord) -> StoreResult<()> {
        let tx = self.conn.transaction()?;
        repos::peers::upsert(&tx, peer)?;
        tx.commit()?;
        Ok(())
    }

    pub fn replace_peers(&mut self, peers: &[PeerRecord]) -> StoreResult<()> {
        let tx = self.conn.transaction()?;
        repos::peers::clear(&tx)?;
        for peer in peers {
            repos::peers::upsert(&tx, peer)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn create_invitation(
        &mut self,
        enrollment_id: EnrollmentId,
        secret: &[u8; 32],
        expires_at_unix_ms: i64,
    ) -> StoreResult<InvitationRecord> {
        let record = InvitationRecord {
            enrollment_id,
            secret_digest: sha256(secret),
            expires_at_unix_ms,
            state: InvitationState::Pending,
            bound_node_id: None,
            created_at_unix_ms: now_unix_ms(),
        };
        let tx = self.conn.transaction()?;
        repos::invitations::insert(&tx, &record)?;
        tx.commit()?;
        Ok(record)
    }

    pub fn bind_invitation(
        &mut self,
        enrollment_id: EnrollmentId,
        secret: &[u8],
        joiner: &PeerRecord,
        now_ms: i64,
    ) -> StoreResult<()> {
        let tx = self.conn.transaction()?;
        let record = repos::invitations::get(&tx, enrollment_id)?
            .ok_or_else(|| StoreError::NotFound("invitation not found".to_owned()))?;

        if record.expires_at_unix_ms <= now_ms {
            repos::invitations::set_state(&tx, enrollment_id, InvitationState::Expired, None)?;
            return Err(StoreError::Corrupt("invitation expired".to_owned()));
        }

        let digest = sha256(secret);
        if digest != record.secret_digest {
            return Err(StoreError::Corrupt("invitation secret mismatch".to_owned()));
        }

        match record.state {
            InvitationState::Pending => {}
            InvitationState::Bound | InvitationState::Consumed => {
                if record.bound_node_id != Some(joiner.node_id) {
                    return Err(StoreError::Corrupt(
                        "invitation already used by another node".to_owned(),
                    ));
                }
            }
            InvitationState::Expired => {
                return Err(StoreError::Corrupt("invitation expired".to_owned()));
            }
        }

        repos::invitations::set_state(
            &tx,
            enrollment_id,
            InvitationState::Consumed,
            Some(joiner.node_id),
        )?;
        repos::peers::upsert(&tx, joiner)?;
        tx.commit()?;
        Ok(())
    }

    pub fn accept_enrollment_snapshot(
        &mut self,
        inviter: &PeerRecord,
        known_peers: &[PeerRecord],
    ) -> StoreResult<()> {
        let tx = self.conn.transaction()?;
        repos::peers::upsert(&tx, inviter)?;
        for peer in known_peers {
            if peer.node_id != inviter.node_id {
                repos::peers::upsert(&tx, peer)?;
            }
        }
        repos::onboarding::set_step(&tx, "enrolled")?;
        tx.commit()?;
        Ok(())
    }

    pub fn list_active_reservations(&self) -> StoreResult<Vec<LocalReservation>> {
        let _ = repos::reservations::delete_expired(&self.conn, now_unix_ms())?;
        repos::reservations::list_active(&self.conn, now_unix_ms())
    }

    pub fn upsert_reservation(&mut self, reservation: &LocalReservation) -> StoreResult<()> {
        let tx = self.conn.transaction()?;
        repos::reservations::upsert(&tx, reservation)?;
        tx.commit()?;
        Ok(())
    }

    pub fn delete_reservation(&mut self, reservation_id: ReservationId) -> StoreResult<()> {
        let tx = self.conn.transaction()?;
        repos::reservations::delete(&tx, reservation_id)?;
        tx.commit()?;
        Ok(())
    }

    pub fn delete_reservations_for_owner(&mut self, owner_node_id: NodeId) -> StoreResult<usize> {
        let tx = self.conn.transaction()?;
        let changed = repos::reservations::delete_owner(&tx, owner_node_id)?;
        tx.commit()?;
        Ok(changed)
    }

    pub fn clear_reservations(&mut self) -> StoreResult<usize> {
        let tx = self.conn.transaction()?;
        let changed = repos::reservations::delete_all(&tx)?;
        tx.commit()?;
        Ok(changed)
    }

    pub fn get_reservation(
        &self,
        reservation_id: ReservationId,
    ) -> StoreResult<Option<LocalReservation>> {
        repos::reservations::get(&self.conn, reservation_id)
    }
    pub fn upsert_model_manifest(&mut self, record: &ModelManifestRecord) -> StoreResult<()> {
        let tx = self.conn.transaction()?;
        repos::models::upsert_manifest(&tx, record)?;
        tx.commit()?;
        Ok(())
    }

    pub fn get_model_manifest(
        &self,
        cache_key: &str,
    ) -> StoreResult<Option<ModelManifestRecord>> {
        repos::models::get_manifest(&self.conn, cache_key)
    }

    pub fn upsert_model_cache_entry(&mut self, entry: &ModelCacheEntry) -> StoreResult<()> {
        let tx = self.conn.transaction()?;
        repos::models::upsert_cache_entry(&tx, entry)?;
        tx.commit()?;
        Ok(())
    }

    pub fn get_model_cache_entry(
        &self,
        entry_id: &str,
    ) -> StoreResult<Option<ModelCacheEntry>> {
        repos::models::get_cache_entry(&self.conn, entry_id)
    }

    pub fn list_model_cache_entries(&self) -> StoreResult<Vec<ModelCacheEntry>> {
        repos::models::list_cache_entries(&self.conn)
    }

    pub fn delete_model_cache_entry(&mut self, entry_id: &str) -> StoreResult<()> {
        let tx = self.conn.transaction()?;
        repos::models::delete_cache_entry(&tx, entry_id)?;
        tx.commit()?;
        Ok(())
    }

    pub fn model_cache_view(&self, root: impl Into<String>, max_bytes: u64) -> StoreResult<ModelCacheView> {
        let (used_bytes, protected_bytes, entry_count, partial_count) =
            repos::models::cache_usage_bytes(&self.conn)?;
        Ok(ModelCacheView {
            root: root.into(),
            used_bytes,
            protected_bytes,
            max_bytes,
            entry_count,
            partial_count,
        })
    }


    fn configure(&self) -> StoreResult<()> {
        self.conn.pragma_update(None, "foreign_keys", "ON")?;
        self.conn.pragma_update(None, "journal_mode", "WAL")?;
        self.conn.pragma_update(None, "synchronous", "FULL")?;
        self.conn.pragma_update(None, "busy_timeout", 5000i32)?;
        Ok(())
    }

    fn migrate(&self) -> StoreResult<()> {
        let version = self
            .conn
            .pragma_query_value(None, "user_version", |row| row.get::<_, i32>(0))?;

        if version > SCHEMA_VERSION {
            return Err(StoreError::NewerSchema {
                found: version,
                supported: SCHEMA_VERSION,
            });
        }

        if version < 1 {
            self.conn.execute_batch(
                r#"
                CREATE TABLE local_identity (
                    id INTEGER PRIMARY KEY CHECK (id = 1),
                    node_id BLOB NOT NULL UNIQUE,
                    mesh_id BLOB NOT NULL,
                    display_name TEXT NOT NULL,
                    certificate_der BLOB NOT NULL,
                    private_key_der BLOB NOT NULL,
                    created_at_unix_ms INTEGER NOT NULL
                );

                CREATE TABLE peers (
                    node_id BLOB PRIMARY KEY,
                    display_name TEXT NOT NULL,
                    certificate_der BLOB NOT NULL,
                    candidates_json TEXT NOT NULL,
                    last_successful_address TEXT,
                    last_seen_unix_ms INTEGER,
                    first_seen_unix_ms INTEGER NOT NULL DEFAULT 0,
                    record_updated_at_unix_ms INTEGER NOT NULL DEFAULT 0,
                    origin TEXT NOT NULL DEFAULT 'direct_peer'
                );

                CREATE TABLE invitations (
                    enrollment_id BLOB PRIMARY KEY,
                    secret_digest BLOB NOT NULL,
                    expires_at_unix_ms INTEGER NOT NULL,
                    state TEXT NOT NULL,
                    bound_node_id BLOB,
                    created_at_unix_ms INTEGER NOT NULL
                );

                CREATE TABLE onboarding (
                    id INTEGER PRIMARY KEY CHECK (id = 1),
                    last_step TEXT NOT NULL
                );

                CREATE TABLE reservations (
                    reservation_id BLOB PRIMARY KEY,
                    deployment_id BLOB NOT NULL,
                    owner_node_id BLOB NOT NULL,
                    amount_json TEXT NOT NULL,
                    state TEXT NOT NULL,
                    purpose TEXT NOT NULL,
                    expires_at_unix_ms INTEGER NOT NULL,
                    created_at_unix_ms INTEGER NOT NULL,
                    updated_at_unix_ms INTEGER NOT NULL
                );
                CREATE INDEX idx_reservations_owner ON reservations(owner_node_id);
                CREATE INDEX idx_reservations_expiry ON reservations(expires_at_unix_ms);

                CREATE TABLE model_manifests (
                    cache_key TEXT PRIMARY KEY,
                    provider TEXT NOT NULL,
                    repository TEXT NOT NULL,
                    revision TEXT NOT NULL,
                    adapter_id TEXT NOT NULL,
                    adapter_version TEXT NOT NULL,
                    model_format TEXT NOT NULL,
                    quantization TEXT,
                    manifest_hash TEXT NOT NULL,
                    canonical_bytes BLOB NOT NULL,
                    created_at_unix_ms INTEGER NOT NULL
                );
                CREATE INDEX idx_model_manifests_revision
                    ON model_manifests(provider, repository, revision);

                CREATE TABLE model_cache_entries (
                    entry_id TEXT PRIMARY KEY,
                    provider TEXT NOT NULL,
                    repository TEXT NOT NULL,
                    revision TEXT NOT NULL,
                    artifact_path TEXT NOT NULL,
                    relative_path TEXT NOT NULL,
                    byte_length INTEGER NOT NULL,
                    range_start INTEGER,
                    range_end INTEGER,
                    etag TEXT,
                    digest_hex TEXT,
                    dtype TEXT,
                    shape_json TEXT,
                    state TEXT NOT NULL,
                    reference_count INTEGER NOT NULL,
                    pinned INTEGER NOT NULL,
                    last_used_at_unix_ms INTEGER NOT NULL,
                    created_at_unix_ms INTEGER NOT NULL
                );
                CREATE INDEX idx_model_cache_last_used
                    ON model_cache_entries(last_used_at_unix_ms);
                CREATE INDEX idx_model_cache_state
                    ON model_cache_entries(state);

                INSERT INTO onboarding (id, last_step) VALUES (1, 'not_enrolled');
                "#,
            )?;
            self.conn.pragma_update(None, "user_version", 4i32)?;
            return Ok(());

        }

        if version < 2 {
            let now = now_unix_ms();
            self.conn.execute_batch(
                r#"
                ALTER TABLE peers ADD COLUMN last_successful_address TEXT;
                ALTER TABLE peers ADD COLUMN last_seen_unix_ms INTEGER;
                ALTER TABLE peers ADD COLUMN first_seen_unix_ms INTEGER NOT NULL DEFAULT 0;
                ALTER TABLE peers ADD COLUMN record_updated_at_unix_ms INTEGER NOT NULL DEFAULT 0;
                ALTER TABLE peers ADD COLUMN origin TEXT NOT NULL DEFAULT 'direct_peer';
                "#,
            )?;
            self.conn.execute(
                r#"
                UPDATE peers
                SET
                    first_seen_unix_ms = CASE
                        WHEN first_seen_unix_ms = 0 THEN ?1
                        ELSE first_seen_unix_ms
                    END,
                    record_updated_at_unix_ms = CASE
                        WHEN record_updated_at_unix_ms = 0 THEN ?1
                        ELSE record_updated_at_unix_ms
                    END
                "#,
                params![now],
            )?;
            self.conn.pragma_update(None, "user_version", 2i32)?;
        }

        if version < 3 {
            self.conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS reservations (
                    reservation_id BLOB PRIMARY KEY,
                    deployment_id BLOB NOT NULL,
                    owner_node_id BLOB NOT NULL,
                    amount_json TEXT NOT NULL,
                    state TEXT NOT NULL,
                    purpose TEXT NOT NULL,
                    expires_at_unix_ms INTEGER NOT NULL,
                    created_at_unix_ms INTEGER NOT NULL,
                    updated_at_unix_ms INTEGER NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_reservations_owner
                    ON reservations(owner_node_id);
                CREATE INDEX IF NOT EXISTS idx_reservations_expiry
                    ON reservations(expires_at_unix_ms);
                "#,
            )?;
            self.conn.pragma_update(None, "user_version", 3i32)?;
        }

        if version < 4 {
            self.conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS model_manifests (
                    cache_key TEXT PRIMARY KEY,
                    provider TEXT NOT NULL,
                    repository TEXT NOT NULL,
                    revision TEXT NOT NULL,
                    adapter_id TEXT NOT NULL,
                    adapter_version TEXT NOT NULL,
                    model_format TEXT NOT NULL,
                    quantization TEXT,
                    manifest_hash TEXT NOT NULL,
                    canonical_bytes BLOB NOT NULL,
                    created_at_unix_ms INTEGER NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_model_manifests_revision
                    ON model_manifests(provider, repository, revision);

                CREATE TABLE IF NOT EXISTS model_cache_entries (
                    entry_id TEXT PRIMARY KEY,
                    provider TEXT NOT NULL,
                    repository TEXT NOT NULL,
                    revision TEXT NOT NULL,
                    artifact_path TEXT NOT NULL,
                    relative_path TEXT NOT NULL,
                    byte_length INTEGER NOT NULL,
                    range_start INTEGER,
                    range_end INTEGER,
                    etag TEXT,
                    digest_hex TEXT,
                    dtype TEXT,
                    shape_json TEXT,
                    state TEXT NOT NULL,
                    reference_count INTEGER NOT NULL,
                    pinned INTEGER NOT NULL,
                    last_used_at_unix_ms INTEGER NOT NULL,
                    created_at_unix_ms INTEGER NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_model_cache_last_used
                    ON model_cache_entries(last_used_at_unix_ms);
                CREATE INDEX IF NOT EXISTS idx_model_cache_state
                    ON model_cache_entries(state);
                "#,
            )?;
            self.conn.pragma_update(None, "user_version", 4i32)?;
        }


        Ok(())
    }
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesh_core::{
        CacheValidationState, ModelCacheEntry, ModelFormat, ModelManifestRecord, PROVIDER_HUGGINGFACE,
    };

    #[test]
    fn schema_v4_persists_manifest_and_cache_entries() {
        let root = std::env::temp_dir().join(format!("mesh-store-model-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = StorePaths::isolated(&root);
        let mut store = Store::open(paths).expect("open store");

        let manifest = ModelManifestRecord {
            cache_key: "huggingface:Qwen/Qwen3-4B:0123456789abcdef0123456789abcdef01234567:adapter=qwen3-dense@1.0.0:fmt=safetensors:quant=none".to_owned(),
            provider: PROVIDER_HUGGINGFACE.to_owned(),
            repository: "Qwen/Qwen3-4B".to_owned(),
            revision: "0123456789abcdef0123456789abcdef01234567".to_owned(),
            adapter_id: "qwen3-dense".to_owned(),
            adapter_version: "1.0.0".to_owned(),
            model_format: ModelFormat::Safetensors,
            quantization: None,
            manifest_hash: "ab".repeat(32),
            canonical_bytes: b"{\"ok\":true}".to_vec(),
            created_at_unix_ms: 1,
        };
        store.upsert_model_manifest(&manifest).expect("upsert manifest");
        let loaded = store
            .get_model_manifest(&manifest.cache_key)
            .expect("get manifest")
            .expect("manifest present");
        assert_eq!(loaded.manifest_hash, manifest.manifest_hash);
        assert_eq!(loaded.canonical_bytes, manifest.canonical_bytes);

        let entry = ModelCacheEntry {
            entry_id: "entry-1".to_owned(),
            provider: PROVIDER_HUGGINGFACE.to_owned(),
            repository: "Qwen/Qwen3-4B".to_owned(),
            revision: manifest.revision.clone(),
            artifact_path: "model.safetensors".to_owned(),
            relative_path: "objects/hf/qwen/model".to_owned(),
            byte_length: 128,
            range_start: Some(0),
            range_end: Some(128),
            etag: Some("\"etag\"".to_owned()),
            digest_hex: None,
            dtype: Some("F16".to_owned()),
            shape_json: Some("[2,32]".to_owned()),
            state: CacheValidationState::Valid,
            reference_count: 1,
            pinned: false,
            last_used_at_unix_ms: 2,
            created_at_unix_ms: 2,
        };
        store.upsert_model_cache_entry(&entry).expect("upsert cache");
        let view = store
            .model_cache_view(store.paths().cache_dir.display().to_string(), 0)
            .expect("cache view");
        assert_eq!(view.used_bytes, 128);
        assert_eq!(view.protected_bytes, 128);
        assert_eq!(view.entry_count, 1);

        let _ = std::fs::remove_dir_all(&root);
    }
}

