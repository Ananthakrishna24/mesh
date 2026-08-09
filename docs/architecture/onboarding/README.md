# Desktop Onboarding

| Field | Value |
|---|---|
| Status | Accepted design; not implemented |
| Canonical for | First-run GUI, one-command startup, and enrollment experience |
| Parent | [Architecture overview](../README.md) |
| Contract | [Enrollment contract](enrollment-contract.md) |
| Decision | [ADR-0005: native Rust desktop app](../../decisions/0005-native-desktop-onboarding.md) |

## Product rule

A user starts one application. The application explains the current step, performs every safe automatic action, and asks only for information that cannot be discovered locally.

The user must not need to run separate networking, GPU, model-provider, or node commands.

## Start experience

### From source

The repository root is configured so development startup is one command:

```text
cargo run --release
```

This builds and starts the `mesh` desktop application. It does not require an npm, Node.js, browser, or web frontend build.

### Distributed application

A packaged release is one native executable or normal operating-system application bundle. Opening it starts the GUI and the local mesh runtime.

## GUI technology

Use `eframe`, the native application framework for `egui`.

Reasons:

- Rust UI and Rust node runtime use one build system.
- It produces a native executable.
- It does not require a JavaScript toolchain.
- It supports Linux, macOS, and Windows.
- It is sufficient for forms, progress, status tables, and guided flows.

The GUI must remain a thin client. Networking, enrollment, hardware scanning, model management, and inference rules stay in their owning Rust crates.

## Process architecture

```text
One `mesh` process
├── eframe UI thread
│   ├── First-run flow
│   ├── Enrollment screens
│   └── Mesh dashboard
│
└── Tokio node runtime
    ├── Node Connector
    ├── Direct Link Manager
    ├── Hardware Scanner
    ├── Peer Store
    ├── Local Resource Manager
    ├── Model Store
    └── Inference modules
```

The UI sends typed commands to the node runtime. The runtime publishes state snapshots and progress events to the UI.

```text
GUI ── UiCommand ──▶ Node Runtime
GUI ◀── UiSnapshot ─ Node Runtime
```

The GUI thread never performs network, disk, model download, or GPU work directly.

Use bounded channels or watch-style state updates. A slow UI must not block the node.

## First launch

```text
START
  │
  ▼
Welcome
  │
  ├── Create a new mesh
  │        │
  │        ▼
  │   Prepare this PC
  │        │
  │        ▼
  │   Mesh dashboard
  │
  └── Enroll this PC
           │
           ▼
      Enter invitation
           │
           ▼
      Automatic setup
           │
           ▼
      Connected dashboard
```

The interface uses one clear primary action on each screen. Advanced details stay collapsed unless an automatic step fails.

## Create the first mesh

The user selects **Create a new mesh**.

The application automatically:

1. Creates and persists the local QUIC certificate and private key.
2. Derives the stable Node ID from the certificate.
3. Creates a Mesh ID.
4. Starts the UDP listener.
5. Detects local and public candidate addresses.
6. Attempts router port mapping.
7. Scans CPU, memory, disks, NVIDIA CUDA, and Apple Metal.
8. Saves the complete local node state transaction.
9. Opens the dashboard.

The success screen says:

```text
This PC is ready.

GPU: NVIDIA RTX ...
Available GPU memory: ...
Connection: Direct connection available

[ Add another PC ]
```

## Enroll another PC

On any connected PC, the user selects **Add another PC**. The application creates one enrollment invitation.

On the new PC, the user selects **Enroll this PC** and uses one of:

1. Paste invitation.
2. Open a `.mesh-invite` file.
3. Scan an invitation QR code when a camera or external scanner is available.
4. Open a registered `mesh://` link in a packaged application.

A short numeric code cannot work without a public lookup service. Because this architecture has no public server, the invitation itself must contain the reachable peer details. The GUI hides the long representation unless the user chooses copy or diagnostics.

The application then automatically:

1. Parses the invitation.
2. Starts the local node runtime.
3. Scans local hardware.
4. Tries every invitation address.
5. Completes the mesh handshake.
6. Receives the current peer list.
7. Opens direct links to reachable peers.
8. Shares the local hardware summary.
9. Runs short network checks.
10. Saves the enrolled state.
11. Shows the connected dashboard.

Canonical fields and failure behavior: [Enrollment contract](enrollment-contract.md)

## User-visible steps

The normal enrollment has four screens:

### 1. Welcome

Plain explanation:

> This application connects this PC directly to your other PCs. It will detect the hardware and network automatically.

Actions:

- **Create a new mesh**
- **Enroll this PC**

### 2. Invitation

Plain explanation:

> On a connected PC, choose “Add another PC.” Copy the invitation or save the invitation file. Paste or open it here.

Actions:

- Paste invitation.
- Open invitation file.
- Back.

### 3. Automatic setup

Show steps instead of technical logs:

```text
✓ Created this PC's identity
✓ Detected NVIDIA GPU
✓ Opened local connection port
✓ Connected to PC A
✓ Received 3 known PCs
• Testing direct links
• Sharing hardware details
```

A user may expand **Technical details** when troubleshooting.

### 4. Ready

Show:

- This PC's name and hardware.
- Number of connected peers.
- Direct connection status.
- Model-provider status.
- **Open dashboard**.

## Dashboard boundary

The first dashboard is intentionally small.

It contains:

- This PC.
- Connected and known PCs.
- Hardware summaries.
- Direct connection state.
- **Add another PC**.
- Model-provider connection state.
- Model deployments when inference is implemented.

It does not expose every internal module or protocol message.

## Model-provider onboarding

Public models require no provider login.

The first proof models, `Qwen/Qwen3-4B` and `Qwen/Qwen3-8B`, are public. Their normal onboarding and download path does not ask for a provider token.

For gated or private models, the GUI checks for an existing local provider credential. For the first Hugging Face adapter it checks the configured application store and standard Hugging Face token locations.

If access is required, show:

```text
This model requires Hugging Face access.

1. Open the Hugging Face access-token page.
2. Create a read-only token.
3. Paste it below.

[ Open token page ]
[ Token __________________ ]
[ Test access ]
```

The application validates the token before saving it in the operating system's credential store. Provider credentials remain local to that PC. They never enter SQLite or enrollment invitations.

If a selected inference node lacks model access, its preparation screen explains the missing access and provides the same guided token step. Public models skip this completely.

## Persistence

Core node state is stored through `mesh-store`, not in UI widget state. Identity, Mesh ID, peers, invitations, model metadata, and onboarding progress use bundled SQLite transactions. Provider tokens use native credential stores and never fall back to plaintext files.

`eframe` persistence may store window size and harmless UI preferences. It is not the source of truth for node identity or mesh state.

Canonical contract: [Persistent state](../system/persistent-state.md)

## Restart behavior

After onboarding, later launches skip the welcome flow.

The application:

1. Loads local identity and peer state.
2. Starts the node runtime.
3. Reconnects to known peers in the background.
4. Refreshes hardware state.
5. Opens the dashboard immediately.
6. Shows connection progress without blocking the interface.

A **Reset this PC** action exists under advanced settings. It explains that reset removes local identity, membership, peers, and deployments before proceeding. Verified model-cache deletion is a separate action.

## Close behavior

For the first version, closing the application stops this node after a graceful shutdown. Background service and system-tray modes are deferred. The interface states this clearly.

## Windows requirement

Windows x64 with NVIDIA CUDA is a required first-class target.

For packaged users, opening the application is the complete startup flow. The user does not install Rust, Visual Studio, or CUDA build tools. A compatible NVIDIA driver remains a hardware prerequisite.

The Windows application must:

- Detect the NVIDIA driver, NVML, and CUDA runtime.
- Explain missing or incompatible components in the GUI.
- Request or guide Windows Firewall permission for direct UDP connections.
- Use the normal Windows application-data location.
- Use Windows credential storage for provider tokens when available.
- Register `.mesh-invite` files and `mesh://` links in the packaged installer.
- Remain usable as a networking-only peer when CUDA initialization fails, while clearly marking compute unavailable.

Native Windows CUDA inference must be proven. WSL-only support is not sufficient. Canonical decision: [ADR-0006](../../decisions/0006-windows-nvidia-required.md).

## Accessibility and wording

- Use short sentences.
- Explain one action per screen.
- Never require users to understand QUIC, NAT, CUDA, or Metal to enroll.
- Show exact technical terms only in expandable details.
- Never show a raw protocol error as the main error message.
- Every failure screen has one recommended next action.

## Source

- [egui and eframe](https://github.com/emilk/egui)
