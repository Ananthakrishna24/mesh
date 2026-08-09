# Goal
Design an extendable decentralized hardware mesh architecture for direct GPU compute across internet-connected PCs.

# Done
- Replaced the abandoned HTML page with an agent-readable architecture knowledge base under `docs/` (`ec12b82`).
- Documented the direct Quinn/QUIC connection algorithm, peer-assisted hole punching, and network limits (`ec12b82`).
- Documented Rust crate boundaries and native NVIDIA CUDA and Apple Metal backend paths (`ec12b82`).

# In progress
- Architecture discussion continues before Rust scaffolding begins.

# Decisions
- Rust is the implementation language.
- Use Quinn QUIC over UDP for direct peer transport.
- Do not use a public gateway, compute relay, permanent controller, or permanent master.
- Use native CUDA and Metal paths behind common Rust boundaries.
- Evaluate Candle first for cross-backend inference; defer the training framework until a real workload is defined.
- Treat `docs/README.md` as the entry point and avoid duplicating canonical facts.
- HTML documentation was rejected in favor of plain Markdown optimized for agents.

# Gotchas
- At least one initial peer must be reachable through IPv6, public IPv4, router mapping, or manual forwarding.
- Peer-assisted hole punching cannot guarantee a path through every CGNAT or firewall.
- The repository rule requires this handoff snapshot even though the user does not want it as project documentation.

# Next
1. Define the control-message wire format and versioning rules.
2. Select router-mapping crates for UPnP, NAT-PMP, and PCP.
3. Decide the first supported host targets beyond Linux CUDA and macOS Metal.
4. Scaffold the Rust workspace only after the remaining connection details are accepted.
