# Rune migration progress

## Current status

Milestone 1 bootstrap implemented: Rune builds, the runtime interaction test
passes, and headless light/dark screenshots and element trees were inspected.
Ricky confirmed the native app works from the interactive desktop session.
The earlier automated display-link failure remains recorded below; normal
shutdown and detailed native input/lifecycle checks have not been confirmed.
The existing GPUI frontend remains the active/default application. No backend
integration or comparative benchmark has been performed.

## Next task

Begin milestone 2: the fixture canvas with cards, edges, pan/zoom, selection,
presentation-only dragging, and a floating workbench. Native startup is
user-confirmed; check normal shutdown and detailed input/lifecycle behavior
during the next native validation.

## Framework selection

- Selected revision: `a8ca769b944598e179f33cc6843629b90e6ca831` (current `dev`
  commit when bootstrapping; both crates use the same Git source).
- Features and platform requirements: [validation](validation.md#rune-configuration-and-prerequisites).
- Host: Apple silicon / Darwin 25.6.0; Rust/Cargo 1.99.1.
- Exact commands: [validation](validation.md#verified-rune-commands).

## Preparation handoff

- Added the brief, milestones, starting behavior map, validation guide, and this
  progress log under `docs/port-info/`.
- Reviewed current desktop behavior, frontend-facing backend documentation,
  workspace/app manifests, and reference source entry points.
- Preserved concurrent GPUI layout/output edits; no application code changed.
- Documentation checks: a Python link/whitespace check passed for all five
  Markdown files and 31 local links, including heading anchors. Scoped
  `git diff --check -- docs/port-info` also passed (new untracked files were
  checked by the Python check).
- App tests/builds/native QA: not run for this documentation-only setup.

## Implementation log

### Milestone 1: bootstrap and headless verification

- Reconciled the behavior map against source entry points, desktop documentation,
  and workspace tests. Keep baseline parity separate from backend-only features.
- Added `crates/waypoints-rune`, a Kit button/counter fixture, a scripted tap,
  and a runtime test. Added the workspace member and Cargo lockfile dependencies
  without changing default members, launch scripts, or GPUI vendor patches.
- `cargo check -p waypoints-rune` passed and resolved the lockfile.
- All six check/test/build/fmt/Clippy/doc commands under “Verified Rune commands”
  passed. Runtime test result: 1 passed, 0 failed.
- Both documented headless commands passed. Artifacts are under
  `target/port-info/rune/` (ignored, reproducible). Visually inspected both PNGs;
  parsed JSON and asserted the rendered counter changes from 0 to 1.
- `cargo check -p waypoints-app --locked` passed with existing GPUI deprecation/
  unused-variable and core dead-code warnings. Full GPUI tests/native QA and
  workspace-wide tests were not run. Concurrent UI edits were not modified.
- Native trace command failed to complete within 20 seconds after reporting
  `display link creation failed: -6661`; the tool killed it. No surviving app
  process was found. Native focus/input/close/reopen remain unverified.
- Scoped diff whitespace checks passed. No performance conclusion is drawn from
  debug-build headless timing logs.
- Follow-up: Ricky reported “It works” after being asked to launch
  `cargo run -p waypoints-rune --locked` from the interactive desktop. Record
  native startup as user-confirmed, not an automated trace pass. Shutdown,
  focus, IME, accessibility, and close/reopen are not implied by that report.

Append concise entries as work proceeds. Keep current status and next task above
up to date; do not make the next agent reconstruct them from the full log.

Each entry should include milestone/task, outcome, changed paths, exact checks
and results, artifact locations, unverified native behavior, and next action.
