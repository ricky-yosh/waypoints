# Rune frontend migration

Status: **cutover complete**. Rune is the canonical `waypoints-app` desktop
frontend; the former GPUI frontend, vendored patches, and vendor update scripts
were removed with explicit user approval. The Rune app passes fixture-canvas
and real-backend checks, including exact diff review, history, settings,
keyboard/focus behavior, previews, native window geometry, and macOS native
menus. The user confirmed the native macOS menu, diff divider, Tab/IME/
VoiceOver, close/reopen, and multi-display checks. Backend tests use an
in-process server and deterministic ACP adapter; no real model agent has run.
Linux/Windows validation and real-agent execution remain unverified. See
[readiness.md](readiness.md).

## Goal and scope

The cutover replaced the GPUI frontend with a canvas-first Rune frontend. The
motivation was fluid canvas interaction and an agent-friendly development loop,
not a proven performance advantage over GPUI Fast. Comparative performance was
not measured.

Keep the workflow engine, persistence, jj operations, ACP integration, and CLI.
The new frontend uses the existing `waypoints_backend::Client`; it must not open
the database or invoke jj independently.

The migration and user-approved full cutover are complete. This packet is the
historical and current verification record for the canonical Rune frontend.

## Read in this order

1. This brief.
2. [Progress and handoff](progress.md): current milestone and next task.
3. [Milestones](milestones.md): ordered work and acceptance gates.
4. [Behavior map](behavior-map.md): final Rune parity checklist and intentional differences.
5. [Validation](validation.md): checks and evidence requirements.
6. [Cutover readiness](readiness.md): completed cutover record, evidence, and
   remaining platform/agent limitations.

Read applicable repository instructions and Rust skills before implementation.
Inspect Rune's actual source and examples before choosing APIs.

## Cutover decisions

- Rune is the canonical frontend at `crates/waypoints-app`; it is the workspace
  default. The former GPUI crate, vendor patches, and vendor scripts are removed.
- Rune and Rune Kit use the same pinned Git commit in the app manifest. The
  revision, platform prerequisites, and reproduction commands are recorded here.
- The app uses only `waypoints_backend::Client`; it does not open the database or
  invoke jj independently.
- Rune's camera/input/motion mechanisms drive the canvas. Per-node presentation
  dragging never mutates dependencies or backend records.
- Domain identities stay stable across selection, reordering, and async updates;
  stale responses do not replace another project or node.
- Rune-only preferences are stored separately and skip writes in headless runs.
  Mutating validation uses isolated state/project directories.
- Respect unrelated/concurrent edits. The behavior map records the final
  reconciliation and intentional differences.

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
