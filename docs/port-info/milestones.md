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
- [x] Create `crates/waypoints-rune`; add it as a workspace member without
  changing `default-members`.
- [x] Pin Rune and Rune Kit to the same commit and update the Cargo lockfile.
- [x] Open a minimal native window (user-confirmed interactive desktop launch).
- [ ] Verify normal native shutdown.
- [x] Add a repeatable fixture mode that needs no backend and cannot mutate
  user projects; render a PNG and dump/inspect its element tree.
- [x] Document exact build, test, native-run, and headless commands.
- [x] Check that the existing GPUI app still builds.

Acceptance: the native window works on the development host, headless output is
inspected, and another agent can reproduce both using documented commands.
Missing native access is a pending gate, not a pass.

## 2. Prove the canvas interaction

- [ ] Show fixture cards and edges with stable node identities.
- [ ] Add smooth pan and pointer-anchored zoom using Rune's live mechanisms.
- [ ] Select and drag cards; edges follow. Dragging changes fixture/presentation
  positions only, not dependencies or backend records.
- [ ] Open/close a floating workbench; check camera-space versus screen-space
  placement, keyboard focus, controls, and text selection under zoom.
- [ ] Exercise a larger graph while the workbench receives synthetic output.
- [ ] Script the important interactions and inspect native responsiveness.

Acceptance: camera-only movement avoids unnecessary view/layout rebuilds, hit
testing follows drawn content, and workbench interaction remains usable.
Record workload size, hardware, build profile, scale, and measured results;
do not claim superiority over GPUI without an equivalent comparison.

## 3. Connect a real vertical slice

- [ ] Open a project, select a workflow, and load its graph through the client.
- [ ] Handle backend unavailable, loading, empty, failure/retry, and stale replies.
- [ ] Open a node, edit/save its assignment, and preserve drafts across navigation.
- [ ] Start/continue an eligible run, monitor structured output, and expose the
  existing cancellation and permission-reply behavior.
- [ ] Keep run outcome, snapshot outcome, readiness, and session eligibility
  distinct; preserve backend refusals.
- [ ] Test project/node switching while requests and live updates are in flight.

Acceptance: an isolated project completes the flow through the real backend;
fixture tests cover errors and races without needing an installed ACP agent.
Real agent execution must be explicitly recorded or left unverified.

## 4. Reach current frontend parity

- [ ] Assignment refresh reconciliation, preview, and follow-up composition.
- [ ] Structured output, tool expansion, replay/truncation notices, tail following,
  and pending permission interactions.
- [ ] Exact-target diff review, file navigation, selection/copy, split/unified,
  wrapping, retained position, and explicit freshness.
- [ ] Run/native revision history, pagination, details, and review handoff.
- [ ] Project/sidebar actions, refresh/retry, settings, theme/interface zoom,
  shortcuts, UI preference persistence, and native lifecycle.
- [ ] Validate narrow windows, larger text, light/dark themes, keyboard-only
  navigation, IME, and accessibility on the supported hosts available.
- [ ] Reconcile every row of the behavior map and record intentional differences.

Acceptance: current frontend workflows have behavior-level coverage, native
checks have evidence, and unresolved gaps are visible. Backend/CLI capabilities
not currently exposed in the GUI are not implicit requirements for this port.

## 5. Cut over (requires explicit user approval)

- [ ] Present parity results, remaining limitations, and native validation.
- [ ] Obtain approval for the specific cutover/removal scope.
- [ ] If a fallback is retained, rename the old package to `waypoints-gpui`;
  rename the Rune package to `waypoints-app` and update binary targets as needed.
- [ ] Update default members, launch scripts, docs, and CI/test commands.
- [ ] Remove the old frontend and GPUI vendor patches only when approved and
  confirmed unused; do not keep them as permanent accidental dependencies.

Acceptance: normal documented commands launch Rune, required checks pass, and
the remaining dependency graph matches the approved migration state.
