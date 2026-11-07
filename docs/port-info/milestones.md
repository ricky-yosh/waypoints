# Migration milestones

Work in order. A checkbox means its acceptance evidence is recorded in
[progress.md](progress.md), not just that code was written.

## 0. Preparation

- [x] Create this migration packet.
- [x] Reconcile the behavior map with current source and existing tests before
  implementation; record any gaps or decisions.

## 1. Boot Rune and establish the verification loop

- [x] Inspect the current hello example, native requirements, headless tooling,
  and hot-reload support; select and record an explicit framework revision.
- [x] Create the Rune crate (now canonical at `crates/waypoints-app`); add it
  as a workspace member without changing `default-members` during development.
- [x] Pin Rune and Rune Kit to the same commit and update the Cargo lockfile.
- [x] Open a minimal native window (user-confirmed interactive desktop launch).
- [x] Verify normal native shutdown (bounded traced window exits cleanly; the
  user confirmed interactive close/reopen at cutover).
- [x] Add a repeatable fixture mode that needs no backend and cannot mutate
  user projects; render a PNG and dump/inspect its element tree.
- [x] Document exact build, test, native-run, and headless commands.
- [x] Check that the existing GPUI app still builds (before removal at cutover).

Acceptance: the native window works on the development host, headless output is
inspected, and another agent can reproduce both using documented commands.
Missing native access is a pending gate, not a pass.

## 2. Prove the canvas interaction

- [x] Show fixture cards and edges with stable node identities.
- [x] Add smooth pan and pointer-anchored zoom using Rune's live mechanisms.
- [x] Select and drag cards; edges follow. Dragging changes fixture/presentation
  positions only, not dependencies or backend records.
- [x] Open/close a floating workbench; check camera-space versus screen-space
  placement, keyboard focus, controls, and text selection under zoom.
- [x] Exercise a larger graph while the workbench receives synthetic output.
- [x] Script the important interactions and inspect native responsiveness.
  Headless scripting passes; the user confirmed the native canvas/workbench
  feel at cutover.

Acceptance: camera-only movement avoids unnecessary view/layout rebuilds, hit
testing follows drawn content, and workbench interaction remains usable.
Record workload size, hardware, build profile, scale, and measured results;
do not claim superiority over GPUI without an equivalent comparison.

## 3. Connect a real vertical slice

- [x] Open a project, select a workflow, and load its graph through the client.
- [x] Handle backend unavailable, loading, empty, failure/retry, and stale replies.
- [x] Open a node, edit/save its assignment, and preserve drafts across navigation.
- [x] Start/continue an eligible run, monitor structured output, and expose the
  existing cancellation and permission-reply behavior (deterministic test ACP
  adapter; no real model/agent execution).
- [x] Keep run outcome, snapshot outcome, readiness, and session eligibility
  distinct; preserve backend refusals.
- [x] Test project/node switching while requests and live updates are in flight.

Acceptance: an isolated project completes the flow through the real backend;
fixture tests cover errors and races without needing an installed ACP agent.
Real agent execution must be explicitly recorded or left unverified.

## 4. Reach current frontend parity

- [x] Assignment refresh reconciliation, preview, and follow-up composition.
- [x] Structured output, tool expansion, replay/truncation notices, tail following,
  and pending permission interactions. The user confirmed native scroll/press
  feel at cutover.
- [x] Exact-target diff review, file navigation, selection/copy, split/unified,
  wrapping, retained position, and explicit freshness (native feel confirmed by
  the user at cutover).
- [x] Run/native revision history, pagination, details, and review handoff.
- [x] Project/sidebar actions, refresh/retry, settings, theme/interface zoom,
  shortcuts, UI preference persistence, and native lifecycle. Native macOS
  menus, lifecycle checks, and the documented shortcuts/previews/preferences
  are in; Linux/Windows keep in-app controls because the pinned Rune menu API is
  macOS-only.
- [x] Validate narrow windows, larger text, light/dark themes, keyboard-only
  navigation, IME, and accessibility on the supported hosts available. The user
  confirmed macOS Tab/IME/VoiceOver, menus, diff divider, close/reopen, and
  multi-display behavior. Linux/Windows remain unverified.
- [x] Reconcile every row of the behavior map and record intentional differences
  (see the cutover reconciliation table in
  [behavior-map.md](behavior-map.md#rune-reconciliation-cutover)).

Acceptance: current frontend workflows have behavior-level coverage, native
checks have evidence, and unresolved gaps are visible. Backend/CLI capabilities
not currently exposed in the GUI are not implicit requirements for this port.

## 5. Cut over (requires explicit user approval)

- [x] Present parity results, remaining limitations, and native validation
  ([readiness.md](readiness.md)).
- [x] Obtain approval for the specific cutover/removal scope: full cutover,
  GPUI removed, no fallback retained.
- [x] Full cutover retained no fallback: the Rune package and binary were named
  `waypoints-app`; the old GPUI package was removed rather than renamed.
- [x] Update default members, launch scripts, docs, and test commands.
- [x] Remove the old frontend, GPUI vendor patches, and vendor refresh scripts
  after the user approved full cutover.

Acceptance: normal documented commands launch Rune, required checks pass, and
the remaining dependency graph matches the approved migration state.
