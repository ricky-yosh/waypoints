# Frontend behavior map

This map was reconciled through the Rune port and the GPUI cutover. The former
GPUI reference implementation and its links were removed; the table below is the
canonical record of the Rune frontend, automated coverage, and intentional
differences. Native macOS checks were user-confirmed at cutover; Linux/Windows
and real-agent execution remain unverified.

## Rune reconciliation (cutover)

Automated coverage is the `waypoints-app` unit/integration suite (30 tests,
run with `cargo test -p waypoints-app --locked`) plus the headless scenes listed
in [validation.md](validation.md). The user confirmed the native macOS menu,
diff divider, Tab/IME/VoiceOver, close/reopen, and multi-display checks feel
correct; the remaining platform limits are recorded per row.

| Capability | Rune implementation | Automated coverage | Difference / remaining gap |
|---|---|---|---|
| Startup and native shell | [`main.rs`](../../crates/waypoints-app/src/main.rs), [`startup.rs`](../../crates/waypoints-app/src/startup.rs), [`preferences.rs`](../../crates/waypoints-app/src/preferences.rs) | Real-backend startup, preferences load/save and invalid/newer-file tests, 120-frame native trace, headless menu dispatch, native geometry save/restore/clamp, HUD icon-label test | Rune's native menu API is macOS-only; Windows/Linux do not get a menu bar from this revision. Native menu placement/key equivalents, window geometry, close/reopen, IME/VoiceOver, and multi-display behavior were user-confirmed on macOS. |
| Projects and workflow selection | [`project.rs`](../../crates/waypoints-app/src/project.rs), [`state.rs`](../../crates/waypoints-app/src/state.rs), [`preferences.rs`](../../crates/waypoints-app/src/preferences.rs) | `project_add_and_remove_use_the_folder_dialog_and_persist`, `keyboard_toggles_sidebar_and_opens_add_project_dialog`, `backend_navigation_assignment_and_stale_replies_use_the_client` | Flat project list plus the active project's workflows, not a nested expandable tree. Refresh/retry applies to the active project; unavailable projects are diagnosed when selected. The sidebar is fixed-width and persisted as shown/hidden (`⌘B`), not resizable. |
| Graph and workbench entry | [`canvas.rs`](../../crates/waypoints-app/src/canvas.rs), [`fixture.rs`](../../crates/waypoints-app/src/fixture.rs), [`workbench.rs`](../../crates/waypoints-app/src/workbench.rs) | `camera_events_skip_the_view_and_keep_the_pointer_anchor`, `hit_testing_follows_a_moved_and_zoomed_camera`, `dragging_a_card_changes_presentation_only_and_edges_follow`, `pointer_hover_previews_a_card_and_clears_on_leave`, `keyboard_opens_workbench_switches_tabs_and_returns_focus` | Cards open by double press or Return on the focused card. Hovering a card for 260 ms opens a screen-space preview; Shift+Return toggles one for the focused card, and Escape closes it first. The preview is informational rather than a press target, unlike GPUI's popup. Node status, dependency direction, selection, retained camera, and screen-space workbench are preserved. |
| Assignment and workbench state | [`workbench.rs`](../../crates/waypoints-app/src/workbench.rs), [`state.rs`](../../crates/waypoints-app/src/state.rs) | `backend_navigation_assignment_and_stale_replies_use_the_client`, `assignment_edit_and_preview_render_draft_and_notice`, `keyboard_opens_workbench_switches_tabs_and_returns_focus`, `settings_overlay_applies_appearance_and_motion_preferences` | Per-node drafts and tab/review context are retained, Edit/Preview and checked save work, refresh reconciles against the saved baseline, and close restores focus. Both frontends keep unsaved drafts on close and show an "Unsaved" state plus the external-saved notice; neither has a blocking close dialog. Compact assignment text clips to a scroll viewport above the footer; expanded preview fills the workbench. Expand/Restore is present. |
| Runs and output | [`workbench.rs`](../../crates/waypoints-app/src/workbench.rs), [`run_output.rs`](../../crates/waypoints-app/src/run_output.rs), [`state.rs`](../../crates/waypoints-app/src/state.rs) | `backend_run_permission_cancel_and_snapshot_use_the_client`, `structured_output_renders_markdown_tools_replay_and_notices`, `large_fixture_stream_is_bounded`, `keyboard_history_and_output_shortcuts_work` | Fresh/continued starts, follow-up, cancellation, permission replies, bounded Markdown/reasoning/tool rows, replay/gap/retention notices, and tail following are ported. Expired-permission replies surface the backend error; there is no separate countdown UI. |
| Exact patch review | [`diff_review.rs`](../../crates/waypoints-app/src/diff_review.rs), [`state.rs`](../../crates/waypoints-app/src/state.rs) | `diff_review_renders_targets_files_modes_and_staleness`, `backend_run_permission_cancel_and_snapshot_use_the_client` | Current/Run/revision targets, exact commit, file statuses, binary/rename parsing, conflict notice, empty/error states, selection/copy, split/unified, wrap, per-file position, and stale-current reload are ported. The changed-files pane is fixed at 190 points and drag-resizable between 140 and 360; the diff takes the remaining width. Syntax rendering is Rune Kit's fallback; no GPUI-specific highlighter parity claim. |
| History | [`history_view.rs`](../../crates/waypoints-app/src/history_view.rs), [`state.rs`](../../crates/waypoints-app/src/state.rs) | `history_views_render_runs_native_and_keep_rows_on_older_page_failure`, `backend_run_permission_cancel_and_snapshot_use_the_client` | Run/native tabs, snapshot outcome, sessions, pinned cursors, older-page retention, review handoff, and freshness with Load newest are ported. A full Refresh replaces the view with loading/error, matching the recorded GPUI limitation. |
| Preferences and appearance | [`preferences.rs`](../../crates/waypoints-app/src/preferences.rs), [`settings.rs`](../../crates/waypoints-app/src/settings.rs) | `preferences_save_and_load_round_trip`, `invalid_or_newer_files_are_not_writable`, `settings_overlay_applies_appearance_and_motion_preferences`, `interface_zoom_scales_the_whole_ui_from_a_larger_layout`, `window_geometry_is_throttled_persisted_and_flushed`, `window_geometry_rejects_invalid_values`, `window_geometry_clamps_to_a_connected_display` | Separate atomic Rune file stores appearance, interface zoom, reduced motion, projects, selected project/workflow, sidebar visibility, and native window origin/size/scale. GPUI's workspace file is not read or written. Restored geometry is validated, clamped to a connected display, or falls back to centered defaults. Headless runs never write preferences. |
| Fixture and native checks | [`main.rs`](../../crates/waypoints-app/src/main.rs) scenes, [`fixtures`](../../crates/waypoints-app/fixtures), [`validation.md`](validation.md) | Headless scenes for canvas, backend flows, structured output, diff, history, settings, interface zoom, project sidebar, keyboard and hover previews; menu scripts for Settings, sidebar toggle, and project refresh; 120-frame native trace; native geometry save/restore/clamp | Native pointer, IME, VoiceOver, close/reopen, multi-display, and menu behavior were user-confirmed on macOS. No Linux/Windows run or real ACP model/agent run has been performed. One intermittent `display link creation failed: -6661` is recorded; immediate retries and the latest trace passed. |
| Backend boundary | [`backend.rs`](../../crates/waypoints-app/src/backend.rs), [`state.rs`](../../crates/waypoints-app/src/state.rs) | `backend_navigation_assignment_and_stale_replies_use_the_client`, `backend_run_permission_cancel_and_snapshot_use_the_client`, `backend_failure_empty_and_retry_states_render` | The Rune worker calls only `waypoints_backend::Client`; no direct database/jj access. Real ACP model/agent execution is unverified; tests use the deterministic adapter. |

## Preserve distinctions, not accidental limitations

- Run completion and snapshot success are separate facts.
- Graph dependencies, native changes, code conflicts, and session eligibility
  must not be inferred from visual position or conflated.
- Historical review remains pinned; do not silently replace it with current code.
- Assignment refresh must not discard an unsaved draft.
- A stale async response must not overwrite the newly selected node/project.
- Closing a workbench does not cancel its run.
- UI errors must preserve actionable backend diagnostics rather than parse
  English messages into invented failure categories.

Known behavior is documented honestly: full History refresh currently replaces
the view with loading/error, while a failed older-page request retains prior
rows. Improving that is possible, but record the intentional change.

## New canvas behavior

Continuous pan/zoom and presentation-only node dragging were migration goals,
not claims about the removed GPUI behavior. Define and test camera anchoring,
selection, screen-space overlays, and input/focus interaction in milestone 2.
Do not silently introduce graph editing or persisted position formats.

Rune fixture coverage (milestone 2): [`fixture.rs`](../../crates/waypoints-app/src/fixture.rs)
holds stable node/edge ids and deterministic positions; [`canvas.rs`](../../crates/waypoints-app/src/canvas.rs)
owns the `Live` camera, hit-tested cards, and presentation-only drag; [`workbench.rs`](../../crates/waypoints-app/src/workbench.rs)
is the screen-space dialog. Runtime tests assert pointer-anchored zoom with no
view commit, hit testing after camera movement, edge endpoints following a
card drag, workbench size/selection/focus under zoom, and bounded synthetic
output. Headless artifacts and native trace evidence are in
[progress.md](progress.md#milestone-2-fixture-canvas-interaction).

Rune backend coverage (milestone 3): [`backend.rs`](../../crates/waypoints-app/src/backend.rs)
wraps only `waypoints_backend::Client`; [`project.rs`](../../crates/waypoints-app/src/project.rs)
is the screen-space project/workflow navigator. The canvas consumes backend
graphs through `fixture::from_workflow`. Assignment drafts are keyed by
workflow/node and saved with `update_node_checked`. The workbench shows run
state separately from snapshot state, bounded ACP event lines, single-use
permission choices, cancellation, and continuation eligibility. In-process
backend tests cover unavailable/empty/retry states, stale graph replies, draft
preservation, checked assignment saves, permission replies, live node switching,
run completion with a saved snapshot, and cancellation. [`run_output.rs`](../../crates/waypoints-app/src/run_output.rs)
renders the shared bounded `ActivityLog` as selectable Markdown/streaming
Markdown, separate reasoning, stable expandable tool/other rows, replay and gap
notices, and tail following with Jump to latest. Exact diff review, run/native
history, and Settings are now ported in [`diff_review.rs`](../../crates/waypoints-app/src/diff_review.rs),
[`history_view.rs`](../../crates/waypoints-app/src/history_view.rs), and
[`settings.rs`](../../crates/waypoints-app/src/settings.rs), with Rune-only
preferences in [`preferences.rs`](../../crates/waypoints-app/src/preferences.rs).
Sidebar project add/remove with persisted project/workflow selection, native
folder selection, automatic history freshness, native menus, geometry clamping,
traffic-light clearance, and labeled icon controls are also ported. Native
macOS interaction/IME/VoiceOver/close-reopen/multi-display checks were
user-confirmed; Linux/Windows and real-agent execution remain unverified. See
[readiness.md](readiness.md).

## Outside baseline parity

The current desktop documentation says workflow/native-change/workspace
provisioning still uses the CLI and history browsing does not restore code.
Do not turn backend restore/rebase/delete/provisioning APIs into new UI features
as part of a mechanical port.

Backend safeguards still apply to every exposed operation. Consult
[frontend-facing backend contracts](../25-backend-frontend-readiness.md) and the
[current status map](../20-core-backend-status.md) when checking scope.

## Coverage recording

As each capability lands, add its Rune source/test paths and native evidence.
Do not mark parity complete from screenshots alone. Resolve discrepancies
between this inventory, current code, and docs before implementing assumptions.
