# Persistent State

| Field | Value |
|---|---|
| Status | Accepted |
| Canonical for | Durable node state, schema migration, cache metadata, and provider credentials |
| Parent | [Node modules](node-modules.md) |
| Decision | [ADR-0010: SQLite state and native credentials](../../decisions/0010-sqlite-state-and-native-credentials.md) |

## Storage boundary

Use SQLite through `rusqlite` with the `bundled` feature for durable structured state. Use one database named `mesh.db` in the operating system's per-user application data directory.

Model artifacts remain normal files in a sibling cache directory. The database stores their identity, size, validation state, and references. It does not store model weight blobs.

Provider tokens use the operating system's credential store through the Rust `keyring` ecosystem. They are never stored in SQLite, configuration files, invitations, logs, or peer messages.

## Crate ownership

Create `mesh-store` as the only crate that executes SQL or accesses provider credentials.

```text
mesh-core       Stable storage-neutral IDs and records
     ▲
     │
mesh-store      SQLite repositories, migrations, credential adapter
     ▲
     │
mesh-node       Owns storage worker and publishes state snapshots
     ▲
     │
mesh-app        Sends commands; never reads SQLite directly
```

Other crates use repository interfaces defined with storage-neutral types. SQLite rows and `rusqlite` types do not leave `mesh-store`.

## Database configuration

Every connection enables:

```sql
PRAGMA foreign_keys = ON;
PRAGMA journal_mode = WAL;
PRAGMA synchronous = FULL;
PRAGMA busy_timeout = 5000;
```

Rules:

- One dedicated storage worker owns the write connection.
- Async tasks send typed storage commands to that worker. They do not block a Tokio executor thread on SQLite work.
- Multi-record state transitions use one transaction.
- The GUI never opens the database.
- Database startup failure prevents enrollment or job execution and shows a guided recovery error. It does not silently start with empty state.

## Migration contract

- The schema version is SQLite `PRAGMA user_version`.
- Migration SQL is embedded in `mesh-store` and committed with the code.
- Migrations run in ascending order inside transactions before the node starts networking.
- A migration updates `user_version` only after its schema and data changes succeed.
- A database newer than the application supports is opened read-only for diagnostics and normal startup stops.
- Failed migrations preserve the last committed schema and produce a backup path in technical details.
- Released migrations are immutable. Fixes use a new migration.

## Durable records

Persist:

| Record | Required data |
|---|---|
| Local identity | Certificate DER, PKCS#8 private key, derived Node ID, creation time |
| Mesh membership | Mesh ID and local display name |
| Peer Store | Node ID, certificate digest, display name, last accepted capabilities |
| Address candidates | Address, kind, source, priority, observed time, expiry, reachability state |
| Invitations | Enrollment ID, secret digest, expiry, bound Node ID, lifecycle state |
| Onboarding | Last completed durable step |
| Hardware snapshots | Stable device identity and last successful capability report |
| Model manifests | Provider, immutable revision, adapter version, manifest hash |
| Model cache entries | Artifact identity, path relative to cache root, byte length, digest, validation state, references |
| Deployments | Placement plan, assignment hash, model identity, durable lifecycle state |
| Reservations | Lease identity and expiry needed to release stale capacity after restart |
| User preferences | Non-secret application settings |

Do not persist as authoritative state:

- Open QUIC connections.
- Current transfer progress.
- In-memory GPU allocations.
- Live request queues.
- Temporary activation tensors.
- Provider tokens.

On restart, persisted deployments and reservations enter recovery states. They are not reported as live until local resources and every required peer confirm them again.

## Identity transaction

First-run identity creation is one transaction:

1. Generate certificate and private key in memory.
2. Derive the Node ID from certificate DER.
3. Insert the identity row.
4. Insert the new Mesh ID or pending enrollment state.
5. Commit.
6. Publish `IdentityReady` only after commit succeeds.

A partial identity is never published. Normal startup fails if the certificate, key, and stored Node ID do not match.

## Invitation transaction

When accepting a new peer, one transaction:

1. Verify the invitation is pending, unexpired, and its secret digest matches.
2. Bind it to the joining Node ID, or verify the existing binding matches.
3. Upsert the peer record and certificate digest.
4. Mark the invitation consumed when enrollment is durable.
5. Commit before publishing enrollment completion.

This makes retries from the same joining node safe and rejects competing use.

## Cache files

- Cache paths are relative to one configured cache root.
- Temporary downloads end with `.partial` and are not valid cache entries.
- Verify length, model identity, tensor metadata, and required digest before atomic rename.
- Commit the valid database entry only after the final file exists.
- Startup removes unreferenced stale partial files after a grace period.
- Active deployment references prevent eviction.
- The database is metadata. A missing or mismatched file becomes `INVALID`; it is never trusted because a row exists.

## Provider credentials

The first credential key is:

```text
service: mesh.model-provider.huggingface
account: default
```

Behavior:

1. Public Qwen3 models do not request a credential.
2. A gated or private model asks for a read-only token.
3. Validate access before saving.
4. Save through the platform credential adapter.
5. If the credential store is unavailable, offer session-only use and explain that the token will not survive restart.
6. Never fall back to plaintext persistence.
7. Deleting a credential removes it from the native store and refreshes provider capability state.

Target stores are Windows Credential Manager, macOS Keychain, and an available Linux Secret Service implementation. Platform-specific adapters remain behind the same `mesh-store` interface.

## Backup and reset

- A diagnostics export excludes private keys, provider tokens, enrollment secrets, and model data.
- Reset identity and leave mesh are explicit destructive GUI actions.
- Reset closes networking, clears peer and deployment state, removes the local certificate and key, and creates nothing new until the user starts onboarding again.
- Cache deletion is a separate action. Leaving a mesh does not silently delete verified model artifacts.
