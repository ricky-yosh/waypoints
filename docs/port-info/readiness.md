# Rune cutover readiness

Status: **cutover complete**. Rune is the canonical `waypoints-app` desktop
frontend. The full cutover scope was explicitly approved and applied: the former
GPUI crate, its vendored patches, and its vendor update scripts were removed;
the Rune package was renamed to `waypoints-app` and is the workspace default.
The backend, core, and CLI contracts were not changed.

## Implemented and verified

- Canvas-first fixture frontend: nine-card and 24-card graphs, connected edges,
  live pan/pointer-anchored zoom, hit testing, selection, presentation-only card
  dragging, screen-space HUD/workbench, keyboard focus, text selection, and
  bounded synthetic output.
- Backend connection only through `waypoints_backend::Client`: project mode via
  `--project`, workflow list/graph loading, unavailable/empty/error/retry states,
  stale-reply token guards, checked assignment saves with per-node drafts, and
  draft preservation across workflow switches.
- Assignment editing: per-node drafts retained across navigation, Edit/Preview,
  checked saves with the saved baseline, external-saved warnings, save/discard,
  and an unsaved-state indicator. Compact workbenches scroll the editor/preview
  above the footer.
- Run lifecycle: fresh start, continue, cancellation, bounded structured ACP
  events, single-use permission replies, separate run/snapshot state, and
  continuation eligibility.
- Structured output: Markdown and streaming Markdown, separate reasoning,
  stable expandable tool/other rows, replay and event-gap notices, bounded
  retention, tail following, and Jump to latest.
- Exact diff review: Current/Run/revision targets, commit copy, parsed exact
  patch, fixed/resizable changed-files pane, line selection/copy,
  unified/split, wrapping, stale-current reload, and review handoff.
- History: saved runs and native revisions, state/snapshot/session details,
  native operation metadata, pinned cursors, Load older, Refresh, retained rows
  on older-page failure, and review handoff.
- Settings: System/Light/Dark appearance, interface zoom, reduced motion,
  `Cmd+,`, Settings from the HUD, and a separate atomic Rune preferences file.
  Invalid or newer preference files are left untouched.
- Project navigation: add/remove projects through the native folder dialog, a
  persisted project list, and persisted selected project/workflow restored at
  launch. Removing a project only removes the sidebar entry.
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
  centered default. Display hot-plug and mixed-scale setups remain unverified.
- Native menus (macOS): Settings…, Add Project…, Close Window, checked Show
  Projects Sidebar, and enabled Refresh Project, plus the standard Edit and
  Window items. Menu actions share the in-app state methods; checked/enabled
  flags update with sidebar/project state. Linux/Windows have no native menu bar
  with this pinned Rune revision.
- Native chrome and UI: the HUD keeps its normal left margin but starts below
  the macOS traffic lights; HUD actions are labeled SVG icon buttons; compact
  assignment text clips above the footer.

## Automated evidence

Run from the repository root:

```sh
cargo test -p waypoints-app --locked
cargo fmt -p waypoints-app -- --check
cargo clippy -p waypoints-app --all-targets --all-features --locked --no-deps -- -D warnings
cargo doc -p waypoints-app --no-deps --locked
cargo check -p waypoints-app --locked
```

Current result: 30 Rune tests pass, including in-process real-backend tests
against temporary databases/sockets and the deterministic
`crates/waypoints-cli/tests/fixtures/acp_agent.py` adapter.
`cargo test --workspace --locked`, `cargo check --workspace --locked`, app
formatting, app Clippy, and app docs pass. Two stale CLI error-text assertions
were updated without changing backend behavior. A 120-frame native
trace passed (119/120 presented, 1 skipped, p50/p95 16.67/16.67 ms, clean exit),
reported no native-menu errors, and saved/restored/clamped window geometry was
verified with seeded preferences.

Headless artifacts under `target/port-info/rune/` include the canvas, workbench,
large graph, scripted tour, backend navigation/assignment, permission/
completion/continuation/cancellation, structured output, diff review, history,
settings, assignment edit/preview, pointer-hover/keyboard previews, project
sidebar, traffic-light HUD, and native trace captures. See
[validation.md](validation.md) for commands and [progress.md](progress.md) for
exact results.

## User-confirmed native checks (macOS)

The user confirmed the following on a native macOS session:

- native menu bar, checked/enabled states, and key equivalents;
- Tab traversal, IME/text composition, VoiceOver/accessibility, close/reopen,
  and multi-display window behavior;
- diff-pane divider feel, traffic-light clearance, HUD icon labels, and the
  compact assignment layout.

These were informal confirmations, not per-step logs. The native trace and
headless artifacts above remain the reproducible evidence.

## Remaining limitations

- Linux and Windows have not been built or run. Rune's native menu API is
  macOS-only at the pinned revision; those platforms use in-app controls.
- Display hot-plug and mixed-scale multi-display arrangements remain unverified.
  The diff-pane divider is pointer-only; keyboard resize is not implemented.
- No real ACP model/agent has been executed; all agent checks use the
  deterministic test adapter.
- No comparative performance measurement was made, and the former GPUI
  frontend is no longer available for a side-by-side benchmark. Workspace-wide
  `cargo fmt --all` and Clippy reach pre-existing backend formatting/MSRV
  findings; the canonical app's strict checks pass.

## Cutover record

The approved full cutover removed the GPUI crate, `vendor/*`, and the vendor
refresh scripts; moved the Rune crate to `crates/waypoints-app`; set it as the
workspace default; renamed the package and binary to `waypoints-app`; updated
`dev`, `renovate.json`, the README, and the documentation map; and regenerated
`Cargo.lock`. No commit or push was performed as part of this cutover step.

The subsequent agent-led workflow/onboarding work is recorded separately in
[progress.md](progress.md) and milestone 6 of [milestones.md](milestones.md).
