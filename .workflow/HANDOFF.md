# Goal
Design an extendable decentralized hardware mesh architecture for direct GPU compute across internet-connected PCs.

# Done
- Added the initial minimal-brutalist HTML architecture document with system, module, join, internet-link, job-owner, and decision views (`976c9fa`).
- Verified the architecture document in dark and light modes and at desktop and mobile widths (`976c9fa`).

# In progress
- Defining the exact direct connection protocol between peers.

# Decisions
- Use equal peer nodes with no public gateway, relay, permanent controller, or permanent master.
- A new node joins through an invite from one known peer and then learns the remaining peers.
- The creator of a job controls only that job.
- Keep the HTML as an extendable technical architecture document, not a landing page.
- Use a minimal-brutalist visual style with module-level explanations.

# Gotchas
- The project directory was not a Git repository, so Git was initialized for the requested commit.
- Some routers and internet providers cannot accept a direct connection without IPv6, port mapping, or successful hole punching.
- The browser screenshot tool intermittently hit its temporary-storage quota; DOM and layout checks still completed.

# Next
1. Agree on the peer transport and direct connection sequence.
2. Add the agreed connection modules and message flow to the architecture diagram.
3. Define node identity and peer-address records.
4. Define disconnect and reconnect behavior.
