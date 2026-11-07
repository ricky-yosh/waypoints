# Rune frontend migration

Status: Rune bootstrap builds and passes headless checks; native startup is
user-confirmed. Detailed native lifecycle validation remains pending.
The GPUI frontend remains the active/default application.

## Goal and scope

Replace the GPUI frontend with a canvas-first Rune frontend. The motivation is
fluid canvas interaction and an agent-friendly development loop, not a proven
performance advantage over GPUI Fast. Comparative performance is unmeasured.

Keep the workflow engine, persistence, jj operations, ACP integration, and CLI.
The new frontend uses the existing `waypoints_backend::Client`; it must not open
the database or invoke jj independently.

This packet defines the migration sequence, not authorization to land changes,
publish anything, delete the old frontend, or add backend-only product features.

## Read in this order

1. This brief.
2. [Progress and handoff](progress.md): current milestone and next task.
3. [Milestones](milestones.md): ordered work and acceptance gates.
4. [Behavior map](behavior-map.md): reference implementation and parity checklist.
5. [Validation](validation.md): checks and evidence requirements.

Read applicable repository instructions and Rust skills before implementation.
GPUI Kit guides apply to GPUI work; do not assume their APIs or dependency rules
apply to Rune. Inspect Rune's actual source and examples before choosing APIs.

## Working decisions

- Add `crates/waypoints-rune` alongside `crates/waypoints-app`. Keep the current
  default package, GPUI executable, development launcher, and vendor patches.
- Depend on Rune and Rune Kit at the same explicit Git commit. Record the chosen
  revision, upstream examples consulted, platform prerequisites, and reproduction
  commands in this packet. Cargo fetches dependencies; a separate framework
  checkout is optional.
- Build the fixture canvas before wiring real backend operations.
- Preserve user-visible behavior and backend safeguards, not GPUI layout code.
  Rune's camera/input/motion mechanisms should drive the new canvas.
- Keep domain identities stable across selection, reordering, and async updates.
  Stale responses must not replace another project's or node's state.
- Extract framework-independent code only when it has a real shared purpose.
  Do not create a universal GPUI/Rune UI abstraction or copy the workflow engine.
- Use separate Rune UI preferences during development. Keep storage and test
  projects isolated from normal user data for mutating validation.
- Respect unrelated/concurrent edits. Re-read the reference implementation before
  porting a capability; this inventory is a starting point, not a frozen snapshot.
- Do not add node-reparenting or other persistent graph mutations merely because
  the canvas supports dragging. Initially, dragging changes presentation only.

## Agent execution loop

For an implementation run:

1. Read the brief and progress log; inspect the first incomplete milestone.
2. Implement one coherent step using current source, not guessed APIs.
3. Run its acceptance checks. Record failures and environment limitations honestly.
4. Update the checklist and progress log with evidence and the next concrete task.
5. Continue when the gate passes. If native validation cannot be performed, mark
   the gate pending; independent work may continue but the milestone is not done.

Stop for unresolved product decisions, backend contract changes, destructive
operations, or actions outside the agreed scope. Follow the active user's
delegation policy; these instructions do not authorize spawning subagents.
Do not commit, merge, or push unless separately requested.

## Upstream references

- [Rune repository](https://github.com/anomalyco/rune)
- [Hello example](https://github.com/anomalyco/rune/tree/dev/examples/hello)
- [Core documentation](https://github.com/anomalyco/rune/blob/dev/crates/rune/README.md)
- [Rune Kit documentation](https://github.com/anomalyco/rune/blob/dev/crates/rune-kit/README.md)

The selected commit is `a8ca769b944598e179f33cc6843629b90e6ca831`.
Use the [pinned hello example](https://github.com/anomalyco/rune/blob/a8ca769b944598e179f33cc6843629b90e6ca831/examples/hello/src/main.rs),
[core documentation](https://github.com/anomalyco/rune/blob/a8ca769b944598e179f33cc6843629b90e6ca831/crates/rune/README.md),
and [Kit source](https://github.com/anomalyco/rune/blob/a8ca769b944598e179f33cc6843629b90e6ca831/crates/rune-kit/src/lib.rs)
for implementation. See [validation](validation.md) for build configuration,
platform requirements, commands, and the pending native gate.
