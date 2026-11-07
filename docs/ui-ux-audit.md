# UI/UX audit

Status: actionable high-confidence batch complete (History split, narrow-window
layout, context, copy, loading/error visibility). Remaining items are deferred
decisions and native person checks.

Scope: the Rune desktop app in `crates/waypoints-app`. The backend/CLI
contracts, onboarding copy, and agent-led workflow behavior are unchanged.
This audit does not claim native usability was verified from headless
renders. Native checks require a person and are recorded separately.

## Method

- Read `docs/05-ui-and-diffs.md`, `docs/28-agent-led-workflows.md`, the
  repository instructions, and the pinned Rune/Rune Kit APIs.
- Reused the existing Changes divider contract (`Cursor::ResizeColumn`,
  `role("separator")`, drag bounds, and no keyboard resizing).
- Added reusable split-pane bounds and divider rendering in
  `crates/waypoints-app/src/split.rs`.
- Rendered and inspected representative 1280×820 and 720×480 scenes in light
  and dark themes, plus 130% interface-zoom scenes.
- Exercised pointer drags, wheel scrolling, node switches, workflow switches,
  copy actions, Escape/settings, and failure updates in headless runtime
  tests.
- Native macOS open/focus and rendering checks from the prior goal still
  apply; this audit's new scroll, drag, and zoom behavior has not been
  person-verified.

Priority means user impact, not implementation effort.

## Fixed in batch 1

### UX-01 (P0) History had no draggable list/details split

- Location: `crates/waypoints-app/src/history_view.rs` runs and native
  revisions layouts.
- Reproduction: open a node workbench, choose History, try to widen the run
  or revision list. Both panes were fixed; the old list also grew with the
  details pane.
- User impact: long run prompts/revision descriptions are hard to read, and
  the list cannot be given more room on wide windows.
- Fix: History now uses the same divider interaction as Changes. The shared
  `split::bounds` helper keeps the list at 160–360 points and the details
  pane at least 200 points; at 130% zoom in a 720×480 window it clamps the
  list to about 269 points. The width is stored in `HistoryPanel` and
  survives node switches.
- Validation: `history_uses_a_draggable_split_between_list_and_details`
  covers runs, native revisions, drag right, drag left, minimum widths, and
  the narrow/zoomed clamp. Headless trees show
  `history-resize` as a 7-point separator with a 270-point list and a
  429-point details pane at 1280×820.

### UX-02 (P0) Changes divider could collapse the diff in a narrow window

- Location: `crates/waypoints-app/src/diff_review.rs`.
- Reproduction: at 130% zoom in a 720×480 window, drag the Changes divider
  fully right. The list stopped at 360 points and left only about 110 points
  for the diff.
- User impact: the diff, path headers, and code become unreadable even though
  the divider appears usable.
- Fix: Changes now uses the shared divider and clamps the file list against
  the remaining workbench width, leaving at least 220 points for the diff.
- Validation: extended
  `diff_review_renders_targets_files_modes_and_staleness` with a 130% zoom
  narrow drag; it verifies both the diff minimum and the 140-point list
  minimum.

### UX-03 (P0) Settings was clipped at large text/narrow windows

- Location: `crates/waypoints-app/src/settings.rs`.
- Reproduction: 720×480 window at 130% interface zoom, open Settings. The
  panel measured about 531 layout points tall in a 369-point content area
  (690 px in the scale-2 capture): title, close button, and lower shortcuts
  were off-screen with no way to scroll.
- User impact: users who need larger text cannot reach the whole settings
  panel or close it with the visible button.
- Fix: the panel is capped to the content height; appearance, zoom, motion,
  shortcuts, errors, and the preferences path live in a scrollable body while
  the title/close row stays fixed.
- Validation: `settings_and_project_panels_fit_the_minimum_window_at_large_zoom`
  asserts the panel stays inside 720×480, `settings-scroll` exists, and a wheel
  gesture moves lower content into view; `settings-zoom` headless scenes were
  visually inspected.

### UX-04 (P0) Project panel fell off-screen at large zoom

- Location: `crates/waypoints-app/src/project.rs`.
- Reproduction: 720×480 at 130% zoom with a project open. The fixed 360-point
  panel began at y=96 and ended below the window; the graph count and part of
  the workflow list were unreachable.
- User impact: users lose the workflow switcher and graph summary exactly when
  space is tight.
- Fix: panel height is derived from the content height and clamped to
  240–360 points. At the tested minimum it is 255 points, keeping the graph
  summary and a scrollable workflow list visible.
- Validation: covered by the same panel-fit test; `project-sidebar-zoom`
  headless scenes were visually inspected.

### UX-05 (P1) Switching nodes lost the Output transcript

- Location: `crates/waypoints-app/src/state.rs`
  (`select_remote_node`, `set_workbench_tab`) and the Output tab.
- Reproduction: finish a run, select another node, then return. The selected
  run still existed, but the transcript was empty until another live poll.
- User impact: users cannot review completed agent output after comparing
  nodes; the app looks like it discarded history.
- Fix: stopped runs reload their bounded transcript from the existing
  `run.events` backend method when Output is opened or a node is revisited.
  This is read-only and never starts/continues a run.
- Validation: extended
  `coordinator_creates_opens_and_runs_work_reflected_by_live_sync` to create
  a second node, switch away, return, and confirm `event-0` is replayed.

### UX-06 (P1) Workflow graph loading/failure could be a blank canvas

- Location: `crates/waypoints-app/src/state.rs` (`select_workflow`,
  `select_project`) and `crates/waypoints-app/src/onboarding.rs`.
- Reproduction: switch workflows or projects, or start with a saved workflow.
  The previous `source_graph` stayed in state while the new graph loaded;
  failures only appeared in the project panel, which can be hidden.
- User impact: a blank canvas with no explanation, and no retry when the
  sidebar is closed.
- Fix: switching workflows/projects clears the previous graph. A missing
  graph now shows a central “Loading workflow…” state or a graph error with
  **Retry graph** and **Choose another project…**. The project panel is not
  required to see or recover from the failure.
- Validation: `workflow_graph_loading_and_failure_stay_visible_without_the_sidebar`
  covers loading, failure, retry affordance, and recovery without the sidebar.

### UX-07 (P1) Failed History refresh deleted usable rows

- Location: `crates/waypoints-app/src/state.rs` History load/update paths and
  `history_view.rs` notice rendering.
- Reproduction: open History with loaded rows, press Refresh while the backend
  request fails. The app cleared rows, selection, and cursor; the tab showed
  only an error.
- User impact: a transient failure destroys the view the user was reading.
- Fix: refresh keeps loaded rows and selection visible, marks the inline
  notice as an error (`notice_error`), and only shows the full error state
  when there was no prior data. Older-page failures also use the error color
  and explicit “Couldn’t load older …” wording. A successful refresh validates
  that the selection still exists and otherwise selects the first row.
- Validation: `failed_history_refresh_keeps_rows_and_selection` covers runs
  and revisions; the existing older-page test covers append failures.

### UX-08 (P1) Copy feedback and native-history copy were inconsistent

- Location: `crates/waypoints-app/src/diff_review.rs`,
  `crates/waypoints-app/src/history_view.rs`, shared state.
- Reproduction: Changes offered **Copy commit** with no confirmation. Native
  History displayed the full commit ID but offered no copy action.
- User impact: users cannot tell whether the copy worked, and must manually
  transcribe revision IDs from History.
- Fix: both actions now copy the full ID and show “Copied commit” beside the
  action for 2.5 seconds. Native revision details gained **Copy commit**.
  Copy actions carry tooltips, and the notice is cleared when the workbench
  tab or node changes so it cannot leak into an unrelated surface.
- Validation: the diff and history tests click the copy actions and assert
  `review-copy-status` / `history-copy-status` plus the shared notice.

### UX-09 (P1) Reconnection status was invisible with the sidebar hidden

- Location: `crates/waypoints-app/src/canvas.rs` HUD.
- Reproduction: hide the project navigator, then let a background refresh
  fail. “Reconnecting” only existed inside the hidden panel.
- User impact: users keep reading stale graph/run state without knowing
  refreshes stopped.
- Fix: the HUD replaces its node/edge summary with a warning-colored
  **Reconnecting** status carrying the error as help while `sync_error` is
  set.
- Validation: the graph loading/failure test asserts `canvas-sync-status` is
  present with the project panel hidden and the node count is not shown.

### UX-10 (P2) Copy/action wording and pointer affordances

- Location: HUD Reset help, project remove control, workbench header,
  clickable diff/history rows, Settings shortcut copy.
- Reproduction/inspection: Reset still said “Reset the fixture camera” in
  backend mode; project removal was a bare `×`; the header said “Backend
  assignment” while viewing Changes or History; selectable rows used an arrow
  cursor.
- User impact: misleading help, weak accessibility, and inconsistent
  affordance.
- Fix: “Reset the canvas view”; project removal is a labeled close icon with
  “Remove project” and an explicit “files are not deleted” tooltip; the header
  says “Backend workflow”; clickable History/diff rows use the pointer cursor;
  the Escape shortcut description includes closing the workbench.
- Validation: app tests continue to pass; labels are present in headless
  trees (`project-remove-*`, history/diff rows).

### UX-11 (P1) Project-folder errors were reported in Preferences

- Location: `project::open_add_dialog`, `State::add_project`, project panel.
- Reproduction: open **Add** and receive a folder-dialog error, or add an
  inaccessible/deleted path while a project is already open. The error was
  stored as `preferences_error`, which only Settings renders; the project
  panel and onboarding showed nothing.
- User impact: the action appears to fail silently, and the error is shown in
  an unrelated Settings panel.
- Fix: folder errors use `project_open_error`; the project panel renders it
  beside the project list, and onboarding already renders it near the drop
  target. Preferences errors remain reserved for preference-file read/write
  failures. A failed add keeps the current project selected.
- Validation: `project_open_errors_use_the_project_surface_not_preferences`
  checks the state separation, current-project preservation, and visible
  `project-open-error` in the project panel.

### UX-12 (P2) Long error text could clip without a full-text path

- Location: onboarding failure details, project/backend/sync errors, diff
  review errors, History error states.
- Reproduction: force a long backend or graph error in a narrow window; the
  visible message is capped at 2–4 lines.
- User impact: the actionable part of the error can be hidden with no way to
  inspect it.
- Fix: capped error/status text now carries the full message as a tooltip.
- Validation: headless scenes still render the capped copy; app tests cover
  the error states. Native hover behavior is not separately verified.

### UX-13 (P1) Interrupted loads could leave a cached node stuck on Loading

- Location: `State::select_remote_node` / cached `ReviewPanel`.
- Reproduction: start a History refresh or Changes review for node A, switch
  to node B before the reply arrives, then return to A. The stale reply is
  correctly discarded, but A's cached panel remained in `Loading` with no
  request in flight, so the tab could show a loading message forever.
- User impact: a node appears permanently busy, and users must retry or
  switch tabs to recover.
- Fix: returning to a cached node reissues any review, runs, or revisions
  request that was left in `Loading`. Stale replies still cannot overwrite
  another node's state.
- Validation: `returning_to_a_node_resumes_interrupted_requests` interrupts
  both a review and a History request with a node switch and verifies the
  returned panel reaches a settled state while keeping its rows.

### UX-14 (P2) History's primary action was clipped in a short details pane

- Location: History run and native revision details.
- Reproduction: 720×480 at 130% zoom; the details pane is about 128 layout
  points tall. The **Review saved snapshot** / **Review this revision** action
  sat after all metadata and was partly below the viewport.
- User impact: the main handoff action required undiscoverable scrolling.
- Fix: the action row (and **Copy commit** for native revisions) now sits
  directly below the title, before prompt/description metadata. Metadata still
  scrolls when it does not fit.
- Validation: the History split test asserts the primary review action is
  fully inside the narrow details viewport; the refreshed 130% scene shows the
  button before scrolling.

## Verified context behavior

- Node switching preserves the workbench tab, History run/revision selection,
  History split width, drafts, review target, diff mode/wrap/position, and
  output-at-bottom state. `switching_nodes_restores_tab_selection_and_split_width`
  covers the observable tab/selection/width path; the existing draft test
  covers assignment text. Selecting another History entry also scrolls the
  details pane back to its top so a new entry does not open mid-document.
- Workflow switching intentionally clears the old graph and selection while
  the new graph loads. This prevents stale content from being mistaken for
  the selected workflow.
- Copy prompt feedback and persistent onboarding controls from the prior goal
  remain unchanged.

## Deferred decisions

### UX-D1 (P2) Workflow switching discards per-workflow node/workbench context

- Location: `State::select_workflow`.
- Reproduction: select a node and open its workbench, switch to another
  workflow, then switch back. No node is selected and the workbench is closed.
- Behavior: returning to a workflow starts with no node selected and the
  workbench closed, even if the user was reviewing a node there.
- Why deferred: remembering selection per workflow changes navigation
  semantics and needs a product decision about whether reopening a workflow
  should also reopen the workbench. The current behavior is explicit and
  avoids showing a stale node; no code change made.
- Possible fix if approved: store the last node id per workflow, validate it
  against the loaded graph, and restore selection (not necessarily the
  workbench).

### UX-D2 (P2) Split dividers are pointer-only

- Location: `crates/waypoints-app/src/split.rs`, Changes and History.
- Reproduction: focus a divider and press Left/Right or Home/End; the width
  does not change. Only pointer dragging works.
- Behavior: both dividers are focusable-role separators but have no keyboard
  width adjustment. This matches the existing Changes divider, so History did
  not invent a different rule.
- Why deferred: adding arrow-key resizing should be designed once for both
  dividers, with focus-visible feedback and a documented step size.
- Possible fix: focusable separator, Left/Right or Shift+Left/Right steps
  (for example 16 points), Home/End for bounds, announced with the accessible
  label.

### UX-D3 (P3) Scrollable Settings body has no visible scrollbar

- Location: Settings at 130% zoom / 720×480.
- Reproduction: open Settings with a short window and look for a scrollbar or
  an indication that lower shortcuts continue below the last visible row.
- Behavior: a headless wheel gesture scrolls the body to lower content, but
  the lower shortcut rows end at the panel edge without a persistent
  scrollbar or “more” cue.
- Why deferred: Rune's scroll presentation is shared with other panes; a
  custom indicator would be a visual-language change. Native wheel/trackpad
  and scrollbar appearance still need a person check.

### UX-D4 (P3) Project panel is very compact at 130% zoom

- Location: project panel at 720×480/130%.
- Reproduction: open a project with several workflows at 130% zoom in the
  minimum window; the workflow list shows about one row before scrolling.
- Behavior: the workflow list is about 36 layout points tall (one row) before
  scrolling. It remains reachable and the graph summary fits.
- Why deferred: further changes require a decision about collapsing the
  project list or changing panel proportions at small sizes.

## Headless evidence

Scenes rendered from `target/debug/waypoints-app` (scale 2, inspected visually):

- `target/port-info/rune/history-runs-split.png` — light, 1280×820, new
  History split.
- `target/port-info/rune/history-runs-split-dark.png` — dark, 720×480.
- `target/port-info/rune/history-runs-zoom-fixed.png` — 720×480, 130%.
- `target/port-info/rune/settings-zoom-fixed.png` — 720×480, 130%, scroll
  body and fixed header.
- `target/port-info/rune/project-sidebar-zoom-fixed.png` — 720×480, 130%.
- `target/port-info/rune/assignment-zoom.png`,
  `diff-review-zoom.png`, `structured-output-zoom.png` — workbench tabs at
  130%.
- `target/port-info/rune/onboarding-no-project-zoom.png` — onboarding at
  130%.

Interaction coverage is in `crates/waypoints-app/src/main.rs`; the full
suite is the source of truth for pass/fail.

## Native evidence and remaining limits

- Native smoke checks (isolated preferences, project, socket, and state):
  `waypoints-app --native --trace --frames 120 --theme dark --project …`
  exited 0 with 0 failed acquires and no error/panic lines. One run presented
  119/120 frames (p50/p95 16.67/16.67 ms); a later run presented 117/120
  (p50/p95 16.67/33.33 ms). Recorded as host/display-link variability, not a
  performance claim.
- No person has verified the new History divider drag, wheel scrolling in
  the Settings body, or the 130% zoom layouts on a native display.
- Mixed-scale/hot-plug displays remain unverified for this batch.
- The audit fixes were validated with headless runtime interactions, rendered
  scenes, and a native startup trace — not by declaring the app universally
  polished.
