use std::time::{SystemTime, UNIX_EPOCH};

use mesh_core::{
    EnrollmentId, InvitationRecord, InvitationState, LocalIdentity, MeshId, NodeId, PeerRecord,
    identity_matches,
};
use rusqlite::Connection;
use sha2::{Digest, Sha256};

use crate::paths::StorePaths;
use crate::repos;
use crate::{StoreError, StoreResult};

pub const SCHEMA_VERSION: i32 = 1;

#[derive(Debug)]
pub struct Store {
    conn: Connection,
    paths: StorePaths,
}

impl Store {
    pub fn open(paths: StorePaths) -> StoreResult<Self> {
        std::fs::create_dir_all(&paths.data_dir)?;
        std::fs::create_dir_all(&paths.cache_dir)?;
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
                    candidates_json TEXT NOT NULL
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

                INSERT INTO onboarding (id, last_step) VALUES (1, 'not_enrolled');
                "#,
            )?;
            self.conn.pragma_update(None, "user_version", 1i32)?;
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
