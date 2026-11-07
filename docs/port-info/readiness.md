# Rune cutover readiness

Status: **not ready for cutover**. The GPUI frontend remains the default
application and no package rename, removal, or launch-script change has been
made. This report records the current evidence and the remaining gaps for a
later user-approved cutover.

## Implemented and verified

- Offline fixture canvas: nine-card and 24-card graphs, connected edges, live
  pan/pointer-anchored zoom, hit testing, selection, presentation-only card
  dragging, screen-space HUD and workbench, keyboard focus, text selection, and
  bounded synthetic output.
- Backend connection only through `waypoints_backend::Client`: project mode via
  `--project`, workflow list/graph loading, unavailable/empty/error/retry states,
  stale-reply token guards, checked assignment saves with per-node drafts, and
  draft preservation across workflow switches.
- Run lifecycle: fresh start, continue, cancellation, bounded structured ACP
  events, single-use permission replies, separate run/snapshot state, and
  continuation eligibility.
- Structured output: Markdown and streaming Markdown, separate reasoning,
  stable expandable tool/other rows, replay and event-gap notices, bounded
  retention, tail following, and Jump to latest.
- Exact diff review: Current/Run/revision targets, commit copy, parsed exact
  patch, file navigation, line selection/copy, unified/split, wrapping,
  stale-current reload, and review handoff from history.
- History: saved runs and native revisions, state/snapshot/session details,
  native operation metadata, pinned cursors, Load older, Refresh, retained rows
  on older-page failure, and review handoff.
- Settings: System/Light/Dark appearance, 85/100/115/130% interface zoom,
  reduced motion, `Cmd+,`, Settings from the HUD, and a separate atomic Rune
  preferences file. Invalid or newer preference files are left untouched.
- Project navigation: add/remove projects through the native folder dialog, a
  persisted project list, and persisted selected project/workflow restored at
  launch. Removing a project only removes the sidebar entry, never project data.
- History freshness: a newly recorded run or native revision marks loaded
  history as fresh and offers Load newest.
- Keyboard: Return opens the focused card, Shift+Return toggles its preview,
  `Cmd+1–4` switch workbench tabs, `R`/`N` switch history, `Cmd+J` jumps to the
  output tail, `Cmd+R` refreshes the current surface, `Cmd+B` shows/hides the
  sidebar, `Cmd+O` adds a project, Escape closes preview/workbench/settings in
  order, and workbench/Settings close restore focus to their openers. The
  Settings sheet lists these shortcuts.
- Graph previews: resting the pointer on a card for 260 ms shows its title,
  status, summary, and dependency count in a screen-space popup; moving away or
  Escape closes it. Shift+Return toggles the same preview for the focused card.
- Native window geometry: content origin, size, and display scale are saved to
  the Rune preferences file at most every 0.5 s and on normal close. Launch
  restores the saved size and position, converting points with the saved scale.
  Invalid/out-of-range geometry is ignored; saved bounds are clamped to the
  connected display they intersect, and fully off-screen bounds fall back to a
  centered default. Headless renders never write preferences.
- Native menus (macOS): Settings…, Add Project…, Close Window, checked Show
  Projects Sidebar, and enabled Refresh Project, plus Rune's standard Edit and
  Window items. Menu actions share the in-app state methods; checked/enabled
  flags update as the sidebar/project state changes.

## Automated evidence

Run from the repository root:

```sh
cargo test -p waypoints-rune --locked
cargo fmt -p waypoints-rune -- --check
cargo clippy -p waypoints-rune --all-targets --all-features --locked --no-deps -- -D warnings
cargo doc -p waypoints-rune --no-deps --locked
cargo check -p waypoints-app --locked
```

Current result: 28 Rune tests pass, including in-process real-backend tests
against temporary databases/sockets and the deterministic
`crates/waypoints-cli/tests/fixtures/acp_agent.py` adapter. The GPUI app check
passes with its pre-existing dependency warnings, and
`cargo test -p waypoints-app --locked` passes 53 tests; no GPUI app code was
changed by the Rune port. The latest 120-frame native trace passed (118/120
presented, p50/p95 16.67/16.67 ms, clean exit), reported no native-menu errors,
and saved/restored/clamped window geometry was verified with seeded
preferences.

Headless artifacts under `target/port-info/rune/` include the canvas, workbench,
large-graph, scripted canvas tour, backend navigation/assignment, permission/
completion/continuation/cancellation, structured-output, diff-review, history,
settings, pointer-hover and keyboard graph previews, project sidebar, native
window-geometry traces, and native trace captures. See [validation.md](validation.md) for
commands and [progress.md](progress.md) for exact results.

## Remaining gaps before parity can be claimed

- Automated keyboard coverage is in place for the documented shortcuts,
  focus return, and pointer/keyboard previews, but native keyboard-only Tab
  traversal, IME behavior, and accessibility are person checks.
- Rune's native menu API installs only on macOS; Windows and Linux get no
  native menu bar from this pinned revision. The menu bar's visual placement,
  key equivalents, and standard Edit/Window behavior need a person on macOS.
- Geometry clamping was verified with the current single-display host; a
  multi-display arrangement, display hot-plug, and mixed-scale positions still
  need a person.
- Native interactive pointer feel, trackpad gestures, IME, clipboard,
  accessibility, minimize/close/reopen, and multi-window behavior still require
  a person on a desktop session. One earlier trace attempt failed with
  `display link creation failed: -6661`; immediate retries and the latest
  120-frame trace passed, so the failure is intermittent and the native gate
  remains unverified beyond startup/pacing/clean exit/geometry save-and-restore.
- Linux and Windows have not been built or run. No real ACP model/agent has been
  executed; all agent checks used the deterministic test adapter.
- No comparative performance measurement against GPUI has been made.

## Native person checks to perform

Launch from an interactive desktop session:

```sh
cargo run -p waypoints-rune --locked
```

Then check and record:

1. Pan with a drag, trackpad scroll, and middle drag; pinch or `Cmd`-scroll zoom
   stays anchored under the pointer.
2. Select, drag, and double-click cards; edges follow without changing saved
   dependencies. Rest the pointer on a card for the preview, move away to close
   it, then click a card, press Shift+Return, Escape to close, and Return to
   open the workbench.
3. Open the workbench, use `Cmd+1–4` and Tab through
   Assignment/Output/Changes/History, select and copy text, and close/reopen
   with Escape and the close/reopen controls. Confirm focus returns to the
   graph card; repeat for Settings (`Cmd+,`).
4. Run a fixture/backend flow with the deterministic adapter, reply to a
   permission, cancel a run, expand a tool row, scroll away from and back to the
   output tail.
5. Review current/run/revision diffs, switch unified/split and wrap, navigate
   files, select/copy lines, refresh a stale current review, and open history.
6. Use `Cmd+B` to hide/show the sidebar and `Cmd+O` to add a project with the
   native folder picker, remove a project, record a new run while History is
   open, confirm Load newest appears, then switch selected project/workflow.
7. Use the native menu bar for Settings, Add Project, Show Projects Sidebar,
   Refresh Project, Close Window, Undo/Cut/Copy/Paste/Select All, and the
   Window menu; confirm checked/enabled states and key equivalents.
8. Move and resize the window, note the size/position, switch
   System/Light/Dark, interface zoom, and reduced motion, quit, relaunch, and
   confirm the separate Rune preference file restores all of them without
   touching the GPUI file. Repeat on a second display, including partially
   off-screen and disconnected-display cases.
9. Check IME composition, accessibility tree, window resize/narrow layout,
   larger text, minimize/close/reopen, and normal shutdown.

## Cutover gate

Cutover remains blocked until the user approves the exact scope and the native
checks above are recorded. Per [milestones.md](milestones.md), any fallback
rename, package/binary rename, default-members change, launch-script change,
GPUI removal, or vendor-patch cleanup must happen only after that approval.
