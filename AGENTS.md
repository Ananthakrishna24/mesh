# Agent Rules

## Source of truth

1. Read the canonical document under `docs/` before changing that area.
2. `docs/implementation/roadmap.md` is canonical for phase order.
3. `docs/implementation/checklist.md` tracks progress only. Update it in the same change as the work it records.
4. Do not copy architecture rules into multiple documents. Update the canonical file and link to it.

## Implementation order

1. Implement phases in roadmap order. Do not parallelize phases.
2. Do not start a phase until its prerequisite decision gates are resolved.
3. Finish the current phase proof before moving to the next phase.
4. Deferred work stays deferred until the roadmap moves it into scope.

## Coding standards

1. Prefer clear names, small modules, and explicit types over comments.
2. Write almost no code comments. Comments are allowed only for non-obvious invariants, safety, or protocol wire constraints that names cannot express.
3. Do not narrate what the next line does.
4. Do not leave `TODO`, placeholder stubs, or dead code in completed work.
5. Keep crate boundaries from `docs/implementation/rust-workspace.md`.
6. Business rules stay in owning crates. `mesh-app` is a thin GUI over typed commands and snapshots.
7. Prefer boring, explicit Rust. Avoid clever abstractions, premature generics, and extra dependencies.
8. Errors are typed and actionable. Do not swallow failures.
9. Do not silently add central services, relays, or permanent masters.

## Verification

1. Prove behavior with the phase proof from the roadmap when one exists.
2. Run the relevant build or smoke path before claiming a phase item complete.
3. Check checklist items only when the named platforms or configurations are actually done.
4. Record partial cross-platform evidence as indented notes under the unchecked item.

## Session handoff

Maintain `.workflow/HANDOFF.md` as a current snapshot with Goal, Done, In progress, Decisions, Gotchas, and Next.
