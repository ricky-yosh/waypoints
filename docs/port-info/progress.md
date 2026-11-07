# Rune migration progress

## Current status

Milestones 1–3 have automated coverage in `crates/waypoints-rune`, and
milestone 4 now covers structured output, exact-target diff review, run/native
history, settings, theme, Rune-only preferences including window geometry, and
pointer/keyboard graph previews. The fixture
canvas keeps its offline mode, and `--project PATH` connects only through
`waypoints_backend::Client`. Backend mode loads workflows/graphs, edits and
checked-saves assignments with per-node drafts, starts/continues/cancels runs,
replies to single-use permissions, and shows run versus snapshot state.
Twenty-eight Rust tests pass, including in-process real-backend tests with a
deterministic ACP adapter. Headless PNGs/trees for structured output, diff
review, history, settings, pointer-hover/keyboard graph previews, project
sidebar, and the earlier canvas/run flows were inspected. A real
model/installed ACP agent has not been run. Sidebar project add/remove with
persisted project/workflow selection, automatic history freshness, the agreed
keyboard shortcuts/focus return, pointer-hover previews, and persisted native
window geometry now work; every behavior-map row has a source/test
reconciliation. Native menu-bar items and display-aware geometry clamping are
also implemented. Complete native interaction/lifecycle checks (Tab traversal,
IME, accessibility, close/reopen), native menu appearance and multi-display
verification, Windows/Linux validation, and real-agent execution remain. The
latest post-geometry 120-frame native trace passed (118/120 presented, p50/p95 16.67/16.67 ms,
clean exit), with no native-menu errors reported; seeded 1100×700 and
partially off-screen windows restored/clamped with matching trace headers. An
earlier attempt hit the intermittent `display link creation failed: -6661`,
which immediate retries cleared. The existing GPUI frontend remains the
active/default application. See
[readiness.md](readiness.md) for the cutover assessment.

## Next task

Run the native person checks, then present the cutover decision for explicit
approval. The remaining work is native-person verification (menu bar, Tab/IME,
accessibility, close/reopen, multi-display geometry), Windows/Linux, and a real
ACP model/agent; any parity issue those uncover still needs a follow-up.
The pending person check remains: launch
`cargo run -p waypoints-rune --locked`, pan/zoom with a trackpad and mouse,
drag a card, press Return and Shift+Return on a focused card, Tab through the
workbench, review a diff, switch history tabs with R/N and Cmd+1–4, toggle the
sidebar with Cmd+B, add a project with Cmd+O, change appearance, select
assignment text, and close/reopen the window.

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

### Milestone 2: fixture canvas interaction

- Added `fixture.rs`, `canvas.rs`, and `workbench.rs` alongside the bootstrap
  `main.rs`. The model has stable integer node ids and edge endpoints; dragging
  mutates only fixture `x/y`, never `edges` or backend data.
- The world element uses Rune `Live<[f32; 2]>` pan and `Live<f32>` zoom with
  `.live_offset(..).live_scale(..).origin(0, 0)`. Wheel pan, ⌘wheel zoom,
  trackpad magnify, blank/middle/Space drags, and zoom controls update those
  values. Camera handlers call `tx.live_only()`.
- Cards are focusable `button` roles, select on press, open the workbench on
  double press, and drag in world points (`Drag::dx / zoom`); `tx.instant()`
  keeps the card under the pointer. Edge paths are rebuilt from the same model
  every commit. Blank press clears selection.
- The workbench is a screen-space `dialog` pinned 24 pt above the bottom edge.
  It follows the selected node, has close/stream/clear controls, selectable
  assignment and output text, a keyboard-focusable footer, and a bounded
  synthetic stream (80 lines maximum). Its width clamps to a 720 pt window.
- Initial default camera is pan `[22, 44]`, zoom `0.88` so all nine fixture cards
  are visible at 1280 × 820. `workbench`, `large-workbench`, and `zoomed` scenes
  are available to headless runs.
- Changed paths: `crates/waypoints-rune/src/{main,fixture,canvas,workbench}.rs`
  and `crates/waypoints-rune/fixtures/canvas-tour.json`.
- `cargo test -p waypoints-rune --locked`: 9 passed, 0 failed. Tests cover
  pointer-anchored zoom with no view commit (`commit_generation` unchanged),
  hit testing after camera movement, presentation-only drag plus edge endpoint
  movement, workbench screen-space geometry under zoom, text selection under
  zoom, keyboard Tab/Space controls, Escape close, narrow-window sizing, and
  the bounded 24-node stream.
- `cargo clippy -p waypoints-rune --all-targets --all-features --locked -- -D
  warnings`, `cargo fmt -p waypoints-rune -- --check`, and
  `cargo doc -p waypoints-rune --no-deps --locked` pass. `cargo check -p
  waypoints-app --locked` passes with the pre-existing gpui-fast deprecation/
  unused-variable and core dead-code warnings. `cargo test -p waypoints-app
  --locked` passes: 53 passed, 0 failed.
- Headless artifacts under `target/port-info/rune/`:
  `canvas-light.png|json`, `canvas-dark.png|json`, `workbench-dark.png|json`,
  `large-workbench-light.png|json`, `narrow-workbench-light.png|json`, and
  `canvas-tour-light.png|json|jsonl`. All PNGs were visually inspected; trees
  show the camera-scaled card rects, screen-space 760 × 328 workbench, seven
  streamed lines after the scripted tour, and the `dialog` role.
- Scripted command:
  `target/debug/waypoints-rune --script
  crates/waypoints-rune/fixtures/canvas-tour.json --snap
  target/port-info/rune/canvas-tour-light.png --tree
  target/port-info/rune/canvas-tour-light.json --trace
  target/port-info/rune/canvas-tour-light.jsonl --theme light --scale 2 --size
  1280x820 --at mark:end`.
- Workload/host for the debug-build observations: Apple M1, macOS 26.6.2,
  Rust 1.99.1, scale 2, 1280 × 820. Default fixture: 1 frame, 1060 instances,
  108 layout nodes. Large workbench scene at `--at 1.4`: 85 frames, 3598
  instances, 366 layout nodes, 77 animating frames at p50 5.4 ms / p95 9.3 ms
  CPU per frame. These are debug-build headless numbers, not a comparison with
  GPUI.
- Native trace rerun: `target/debug/waypoints-rune --native --trace
  target/port-info/rune/native-canvas-120.jsonl --frames 120 --theme dark --size
  1280x820` printed 120 frames, 119 presented, 1 skipped, interval p50/p95
  16.67/16.67 ms, max 33.33 ms, lag p50/p95 16.67/16.67 ms, and exited normally.
  A 30-frame light run also completed. The earlier automated display-link
  failure therefore did not recur on this host. Interactive pointer feel,
  trackpad gestures, IME, accessibility, minimize/close/reopen, and Linux/
  Windows remain unverified.

### Milestone 3: backend vertical slice

- Added `backend.rs`, a small worker around `waypoints_backend::Client` with
  request tokens. It never opens the database and never invokes jj. Commands
  cover connect, workflow list, graph load, checked assignment save, workflow
  runs, start/continue/cancel, bounded event polling, permission reply, and
  continuation assessment. Results are applied only when their token and
  workflow/node still match.
- Added `project.rs`, a screen-space project/workflow navigator with Connected/
  Connecting/Unavailable states, workflow retry, empty-project copy, graph retry,
  and node/edge counts. `--project PATH`, `--workflow ID`, and `--agent PATH`
  (or the corresponding environment/`--flag=value` forms) select backend mode;
  no project argument keeps fixture mode offline.
- Backend graphs are converted to the same canvas model by
  `fixture::from_workflow`, preserving saved ids, prompts and connections while
  deriving presentation positions. Cards show live run status when available.
- The workbench now has a real `TextArea` assignment editor with Save and Load
  saved actions, per-workflow/node drafts, checked save via
  `update_node_checked`, clean-draft refresh reconciliation, and unsaved-draft
  retention. Backend output shows run lifecycle, separate snapshot outcome,
  bounded structured ACP event lines, cancellation, single-use permission
  choices, and continuation eligibility plus a follow-up composer.
- Changed paths: `crates/waypoints-rune/src/{backend,project,state,startup,activity,main,fixture,canvas,workbench}.rs`,
  `crates/waypoints-rune/Cargo.toml`, `Cargo.lock`, and
  `crates/waypoints-rune/fixtures/backend-tour.json`. State, startup, and
  activity helpers are split out so no non-test module exceeds the Rune-style
  800-line guidance.
- `cargo test -p waypoints-rune --locked`: 12 passed, 0 failed. The two live
  backend tests start `waypoints_backend::serve` in-process against a temporary
  database/socket and use the deterministic `crates/waypoints-cli/tests/fixtures/acp_agent.py`
  adapter. They cover workflow selection, graph loading, a stale graph reply
  after switching workflows, draft preservation across workflow switches,
  checked assignment persistence, permission reply, run completion with a saved
  snapshot, a second run cancelled while awaiting permission, and switching
  nodes while a live permission is pending. Separate tests cover unavailable/
  empty/retry UI states and all earlier fixture canvas behavior.
- `cargo clippy -p waypoints-rune --all-targets --all-features --locked
  --no-deps -- -D warnings`, `cargo fmt -p waypoints-rune -- --check`, and
  `cargo doc -p waypoints-rune --no-deps --locked` pass. `--no-deps` is required
  because the path dependency `waypoints-core` carries a pre-existing
  `ensure_session_after_restore` dead-code warning that `-D warnings` would
  otherwise turn into an error. `cargo check -p waypoints-app --locked` still
  passes with the existing GPUI dependency warnings.
- Headless backend artifacts (isolated temp database/project, not user data):
  `backend-light.png|json`, `backend-tour-light.png|json|jsonl`,
  `backend-run-open.png|json`, `backend-run-start.png|json`,
  `backend-run-complete.png|json`, `backend-run-continued.png|json`, and
  `backend-run-cancelled.png|json`. The screenshots were inspected for the
  project panel, remote graph, saved assignment card update, permission card,
  completed run with saved snapshot and continuation form, continuation result
  (`Recalled: hello`), cancellation, and preserved event output.
- Manual headless backend commands used an isolated `XDG_STATE_HOME` plus the
  CLI to create a jj project, workflow, node change and workspace. The app was
  then run with `--project ... --workflow 2 --agent ...` and scripts under
  `crates/waypoints-rune/fixtures/` (assignment tour) or temporary run scripts
  for permission/continue/cancel. The in-process tests reproduce the important
  transitions without requiring a separately installed agent.
- Limits: no real model or installed ACP agent was executed. Output is one
  text line per ACP event; Markdown/reasoning grouping, stable expandable tool
  rows, replay/truncation presentation, tail-follow controls, diff review,
  history, settings, sidebar add/remove, and preference persistence remain
  milestone 4 work. Native backend trace:
  `target/debug/waypoints-rune --project ... --workflow 1 --native --trace
  target/port-info/rune/native-backend-final.jsonl --frames 120` completed with
  120 frames, 118 presented, 2 skipped, p50/p95 16.67/16.67 ms. Interactive
  native input, IME, clipboard, accessibility, and close/reopen remain pending
  person checks.

### Milestone 4: structured run output

- Reused the framework-independent bounded
  `waypoints_backend::activity::ActivityLog` instead of inventing a second
  transcript model. `State` now retains `ActivityLog`, expanded tool-row ids,
  and tail-following state; event gaps force a stream boundary.
- Added `run_output.rs`. Assistant chunks render as selectable Markdown and
  streaming Markdown; reasoning is separated and muted; tool calls merge by
  `toolCallId` and origin into stable expandable rows with status and summary;
  other ACP updates are stable expandable JSON rows. Replay boundaries, event
  gaps, and retention trimming are shown as notices outside the Markdown.
- The transcript viewport sticks to the bottom, reports when it is away from the
  tail, and shows `Jump to latest`; the kind of output follows the existing
  permission/run controls without changing backend contracts.
- Added the `structured-output` and `structured-output-expanded` headless scenes
  plus `structured_output_renders_markdown_tools_replay_and_notices`, covering
  Markdown text, stable tool expansion, replay/gap notices, and the
  jump-to-latest affordance.
- `cargo test -p waypoints-rune --locked`: 13 passed, 0 failed.
  `cargo fmt -p waypoints-rune -- --check`,
  `cargo clippy -p waypoints-rune --all-targets --all-features --locked
  --no-deps -- -D warnings`, and `cargo doc -p waypoints-rune --no-deps
  --locked` pass.
- Headless artifacts: `target/port-info/rune/structured-output-light.png|json`,
  `structured-output-expanded-light.png|json`, and
  `structured-output-top-light.png|json`. The PNGs were inspected: expanded
  tool rows, reasoning, Markdown prose/heading, replay and event-gap notices,
  plan rows, and Jump to latest after scrolling away from the tail are visible.
  Trees confirm Markdown text, `run-tool-4`, `run-tool-details-4`,
  `run-replay-notice-1`, and `run-gap-notice`.
- Limits: native scroll feel and the `Jump to latest` press need person
  confirmation. Truncation notice rendering is wired to `ActivityLog` retention
  but not separately screenshot-tested with a >128 KiB payload. Exact diff
  review, history, settings/preferences, keyboard parity, IME/accessibility,
  and the final cutover readiness report remain pending.

### Milestone 4: diff review, history, and settings

- Added workbench tabs (Assignment, Output, Changes, History) and an
  Expand/Restore control. Backend workbench state, tab, review target, diff
  state, history rows, and per-file position are retained per workflow/node.
- Added exact-target diff review through `review_node`, `review_run`, and
  `review_node_revision`, parsed into Rune Kit `DiffState`/`DiffView`: explicit
  Current/Run/revision targets, commit copy, file list navigation, line-number
  selection/copy, unified/split, wrapping, stale-current notice/reload, binary
  and rename handling from Kit's parser, and review handoff from history.
- Added run and native history from `node_run_page`/`node_revisions`: separate
  tabs, run state and snapshot outcomes, session references, native operation
  timestamps/predecessors/current heads/run references, opaque pinned cursors,
  Load older, Refresh, and retention of loaded rows when an older-page request
  fails. History rows hand a saved run or exact revision to the Changes tab.
- Added Rune-only preferences at
  `waypoints/rune-workspace.json` under the state directory (or
  `WAYPOINTS_RUNE_PREFERENCES_FILE`). Loading tolerates missing files and leaves
  invalid/newer files untouched; saving is atomic via a temporary file.
  Settings control System/Light/Dark appearance, interface zoom, and reduced
  motion. The GPUI preference file is never read or written. The project panel
  adds/removes projects through Rune's native folder dialog, keeps a project
  list and selected project/workflow in the Rune preferences file, and restores
  the last selection at launch. Loaded history now announces a newer run or
  native revision with a Load newest action.
- Added Settings to the floating HUD and `Cmd+,`; Escape closes Settings before
  the workbench. Diff `j`/`k`/`]`/`[` and copy/selection keys route through
  `DiffState::key`.
- Changed paths: `src/{backend,state,diff_review,history_view,preferences,settings,project,workbench,canvas,main,startup}.rs`,
  `Cargo.toml` (including Rune's `native-file-dialog` feature), and
  `Cargo.lock`.
- `cargo test -p waypoints-rune --locked`: 21 passed, 0 failed (at this step).
  `cargo fmt -p waypoints-rune -- --check`,
  `cargo clippy -p waypoints-rune --all-targets --all-features --locked
  --no-deps -- -D warnings`, and `cargo doc -p waypoints-rune --no-deps
  --locked` pass.
- Headless artifacts inspected: `diff-review-light.png|json`,
  `history-runs-light.png|json`, `history-native-light.png|json`,
  `settings-light.png|json`, `settings-dark-check.png|json`, plus the structured
  output captures. Trees show review files/commit/diff, history rows/details
  and review handoff buttons, and settings appearance/motion controls.
- Interface zoom is implemented by laying the UI out in a larger logical
  window and scaling it back to the physical window, so text rasterizes at the
  scaled size. Settings offers 85/100/115/130%; the headless
  `interface-zoom` scene and test cover layout/drawn geometry.
- Assignment Markdown preview is implemented from the unsaved draft alongside
  Edit mode.
- Limits: full keyboard parity, IME/accessibility, and native close/reopen
  remain pending. No real ACP model/agent or Linux/Windows host has been
  exercised. A native trace attempt after the sidebar work failed with
  `display link creation failed: -6661` and was stopped after the tool timeout;
  immediate 10-frame and 120-frame retries passed (119/120 presented, p50/p95
  16.67/16.67 ms) and exited normally. Native interactive checks still need a
  person.

### Milestone 4: keyboard parity and behavior reconciliation

- Added Return to open the focused graph card and Shift+Return to toggle a
  screen-space graph preview; Escape closes preview, then workbench, then
  selection. Workbench and Settings close restore focus to their openers
  through Rune `focus_return`.
- Added `Cmd+1–4` workbench tabs (opening the workbench when needed), History
  `R`/`N` run/native switch, `Cmd+J` output tail, context-sensitive `Cmd+R`
  (review/history/output surface or project refresh), `Cmd+B` sidebar
  visibility, and `Cmd+O` native add-project dialog. Sidebar visibility is
  stored in the Rune preferences file and exposed as a HUD toggle.
- The Settings sheet documents Settings, Escape, Return, Shift+Return, tabs,
  history, diff navigation, output, refresh, zoom, project, and add-project
  keys. The canvas hint mentions keyboard opening.
- Added a conflict notice to exact-target review; fixed the reduced-motion
  switch's duplicate label; reconciled all nine behavior-map rows with Rune
  source/test paths and intentional differences in
  [behavior-map.md](behavior-map.md#rune-reconciliation-milestone-4).
- Changed paths: `src/{canvas,state,project,settings,main,preferences}.rs` and
  the docs listed above.
- New tests: `keyboard_opens_workbench_switches_tabs_and_returns_focus`
  (preview, Return, Cmd+2, stream click focus, Escape focus return),
  `keyboard_history_and_output_shortcuts_work` (R/N, Cmd+J),
  `keyboard_toggles_sidebar_and_opens_add_project_dialog` (Cmd+B, Cmd+O,
  folder dialog), and Settings shortcut-sheet/focus-return assertions.
- `cargo test -p waypoints-rune --locked`: 24 passed, 0 failed (at this step).
  `cargo fmt -p waypoints-rune -- --check`,
  `cargo clippy -p waypoints-rune --all-targets --all-features --locked
  --no-deps -- -D warnings`, and `cargo doc -p waypoints-rune --no-deps
  --locked` pass.
- Headless artifacts inspected: `graph-preview-light.png|json` (keyboard
  preview beside the focused card) and `settings-shortcuts-light.png|json`
  (full shortcut sheet) plus the prior captures.
- Latest native trace after these changes passed: 120 frames, 119 presented,
  1 skipped, p50/p95 16.67/16.67 ms, clean exit
  (`target/port-info/rune/native-canvas-keyboard.jsonl`).
- Limits at this step: pointer-hover graph previews and persisted window
  geometry were not yet implemented; the next section adds them. GPUI menus
  remain unported. Native Tab traversal, IME, accessibility, and close/reopen
  still need a person. No Linux/Windows or real ACP model/agent run has been
  performed.

### Milestone 4: hover previews and window geometry

- Node cards now preview on pointer rest (260 ms enter, 90 ms leave) as well as
  `Shift+Return`. The preview is placed beside the card, switches to the left or
  above/below when the window edge is close, and closes on leave, canvas press,
  selection, workbench open, or Escape. `Shift+Return` toggles a keyboard
  preview. The preview is informational, not a press target.
- Native window geometry is observed through the root `on_screen_layout` rect:
  content origin in screen points, content size, and display scale. Geometry is
  written to the separate Rune preferences file no more than once every 0.5 s
  and flushed on normal close. Headless `--snap`/`--tree`/script runs do not
  observe or save it.
- Launch restores the saved content size through `AppSpec::size` and the saved
  position through `WindowOptions::position`, converting points to desktop
  pixels with the saved scale. Invalid/non-finite/out-of-range geometry is
  discarded during load; no display-connectivity clamping is implemented yet.
- New tests: `pointer_hover_previews_a_card_and_clears_on_leave`,
  `window_geometry_is_throttled_persisted_and_flushed`, and
  `window_geometry_rejects_invalid_values`.
- `cargo test -p waypoints-rune --locked`: 27 passed, 0 failed (at this step).
  `cargo fmt -p waypoints-rune -- --check`,
  `cargo clippy -p waypoints-rune --all-targets --all-features --locked
  --no-deps -- -D warnings`, and `cargo doc -p waypoints-rune --no-deps
  --locked` pass.
- Headless artifacts: `hover-preview-light.png|json` (pointer script over
  node 4) and refreshed `graph-preview-light.png|json`,
  `settings-shortcuts-light.png|json`, `project-sidebar-light.png|json`, and
  `interface-zoom-light.png|json`.
- Native evidence: a 120-frame trace wrote
  `target/port-info/rune/native-geometry-final.json` with
  `{"x":80.0,"y":44.0,"width":1280.0,"height":820.0,"scale":2.0}` and the
  trace header reported 1280×820 at scale 2. A seeded geometry of
  `{x:120,y:80,width:1100,height:700,scale:2}` launched with a 1100×700 trace
  header and retained that geometry. A headless `--snap` run with a disposable
  preferences file left its saved geometry unchanged.
- Limits at this step: saved positions/sizes were not yet clamped to connected
  displays, and the native menu bar was not yet ported. The next section adds
  both. Native Tab traversal, IME, accessibility, close/reopen, and multi-display
  geometry still need a person.

### Milestone 4: native menus and display-aware geometry

- Replaced the plain `App::run` entry point with `run_windows_with`, whose
  display factory supplies the connected displays before the window opens.
- Added macOS native menu items for Settings… (`Cmd+,`), Add Project… (`Cmd+O`),
  Close Window, Show Projects Sidebar (`Cmd+B`, checked), and Refresh Project
  (`Cmd+R`, enabled when a project is selected). `App::standard_menus()` adds
  the standard Edit and Window items (Undo/Redo/Cut/Copy/Paste/Select All,
  Minimize/Close). Menu item actions use the same state methods as the in-app
  controls; sidebar checked/refresh-enabled flags refresh through
  `AppHandle::set_menu` when state changes.
- Saved window geometry is clamped to the connected display it intersects:
  oversized windows shrink to the display, partially visible windows move fully
  on-screen, and geometry that touches no display falls back to the centered
  default. Display bounds use the Rune display list; macOS uses native point
  bounds and other platforms convert from desktop pixels with the saved scale.
- Headless runs now skip all preference writes, so snapshot/script/menu checks
  cannot mutate the user's Rune preferences. Runtime tests still exercise
  preference persistence because `cargo test` is not a headless render process.
- Added fixture scripts `fixtures/menu-settings.json`,
  `fixtures/menu-sidebar.json`, and `fixtures/menu-refresh.json`.
- `cargo test -p waypoints-rune --locked`: 28 passed, 0 failed.
  `cargo fmt -p waypoints-rune -- --check`,
  `cargo clippy -p waypoints-rune --all-targets --all-features --locked
  --no-deps -- -D warnings`, and `cargo doc -p waypoints-rune --no-deps
  --locked` pass. `cargo test -p waypoints-app --locked`: 53 passed.
- Headless menu checks:
  `menu-settings-light.json` contains `settings-panel`;
  `menu-sidebar-light.json` has no `project-panel` and contains
  `project-toggle`; `menu-refresh-light.json` contains `workflows-error` and
  `workflows-retry`. The disposable preferences geometry was unchanged by all
  three runs.
- Native geometry checks: a fresh 120-frame run saved
  `{x:80,y:44,width:1280,height:820,scale:2}` with a 1280×820 trace header.
  A seeded `{x:1300,y:100,width:1100,height:700,scale:2}` launched at
  1100×700 and was clamped to `{x:340,y:100,...}`. A seeded
  `{x:30000,y:30000,...}` fell back to the centered 1280×820 window. Traces:
  `native-canvas-menu-final.jsonl`, `native-canvas-clamped.jsonl`,
  `native-canvas-offscreen.jsonl`.
- Limits: Rune's native menu API is macOS-only; Windows/Linux do not get a menu
  bar from this pinned revision. The native menu bar's visual placement and key
  equivalents still need a person; display clamping was verified on this host's
  single display, not a multi-display arrangement. Native Tab/IME/accessibility/
  close-reopen and real-agent/Linux/Windows checks remain.

Append concise entries as work proceeds. Keep current status and next task above
up to date; do not make the next agent reconstruct them from the full log.

Each entry should include milestone/task, outcome, changed paths, exact checks
and results, artifact locations, unverified native behavior, and next action.
