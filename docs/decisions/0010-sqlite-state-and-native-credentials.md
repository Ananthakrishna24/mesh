# ADR-0010: SQLite State and Native Credentials

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-08-09 |
| Owners | Architecture discussion |

## Context

A node must preserve identity, membership, peers, invitations, cache metadata, and recovery state across crashes and restarts. The desktop application must behave the same on Windows, Linux, and macOS. Provider tokens need a different boundary from normal state.

## Decision

Use SQLite through `rusqlite` with bundled SQLite. Store one `mesh.db` file in the per-user application data directory. Enable foreign keys, WAL mode, `synchronous=FULL`, and a five-second busy timeout.

Create a `mesh-store` crate. It owns SQL, migrations, a dedicated blocking storage worker, model-cache metadata, and provider credential adapters. Other crates do not access SQLite directly.

Use `PRAGMA user_version` with embedded, transactional, immutable migrations.

Store provider tokens through native credential-store adapters from the Rust `keyring` ecosystem. Never fall back to plaintext persistence. Public models require no credential.

Canonical contract: [Persistent state](../architecture/system/persistent-state.md)

## Rejected: JSON files for authoritative state

Several related records must change atomically. Rewriting independent JSON files can leave identity, invitation, peer, and deployment state inconsistent after a crash.

## Rejected: application UI persistence as core storage

Eframe persistence is suitable for window and display preferences. It is not the source of truth for networking, identity, model, or lease state.

## Rejected: model blobs inside SQLite

Large immutable model files are better handled as validated cache files. Storing them in SQLite increases database size and makes partial downloads and eviction harder.

## Rejected: provider tokens in SQLite

A normal database backup or diagnostics copy must not contain provider credentials. Native credential stores provide the correct platform boundary.

## Consequences

- Bundled SQLite adds compile time but avoids a system SQLite installation requirement.
- The node waits for migrations before opening network connections.
- SQL work does not block async network tasks or the GUI thread.
- Linux credential persistence depends on an available Secret Service. Session-only access remains possible when it is absent.
- Backups must keep the SQLite database, WAL state when live, certificate, and key consistent; the application should create backups through its storage layer rather than copying an open database.
