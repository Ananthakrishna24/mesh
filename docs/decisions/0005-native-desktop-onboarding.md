# ADR-0005: Native Rust Desktop Onboarding

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-08-09 |
| Owners | Architecture discussion |

## Context

Users should build and start one application. First-run onboarding must create or enroll a PC without separate network, GPU, node, or provider commands.

The project is implemented in Rust. A web frontend would add a second build toolchain and more startup parts.

## Decision

Build one native `mesh` desktop application with `egui` through `eframe`.

The application starts both:

- The native GUI on the main thread.
- The Tokio mesh-node runtime in the same process.

From the repository root, development startup is:

```text
cargo run --release
```

Packaged users open one executable or application bundle.

The GUI communicates with the node runtime through typed commands and state snapshots. It does not implement networking or compute rules itself.

Canonical design: [Desktop onboarding](../architecture/onboarding/README.md)

## Why eframe

- Native Rust application.
- One Cargo build system.
- No npm or JavaScript frontend build.
- Cross-platform desktop support.
- Sufficient controls for onboarding, progress, tables, and status.
- Direct `run_native` application entry point.

## Rejected: command-driven onboarding

A CLI may exist later for automation. It is not the primary onboarding path. Users should not manually run separate discovery, port, hardware, or enrollment commands.

## Rejected: Tauri as the first GUI

Tauri is capable, but a normal web frontend adds JavaScript package and frontend build concerns. The first UI does not need browser rendering or a web framework.

## Rejected: local web server as the first GUI

Starting a daemon and then opening a browser creates two visible parts and additional port and lifecycle behavior. The accepted experience is one native application.

## Consequences

- `mesh-app` becomes the default runnable package.
- Core state belongs to the node runtime, not UI persistence.
- Closing the first-version application stops the node.
- Background service and tray modes remain deferred.
- Packaged releases still need normal per-platform build and signing work later.
- Linux source builds may require normal native window-system development packages; no application-specific setup command is added.

## Source

- [egui and eframe](https://github.com/emilk/egui)
