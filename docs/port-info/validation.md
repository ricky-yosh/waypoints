# Validation and evidence

## Current state

The `waypoints-app` package has an offline fixture canvas and an opt-in
backend mode driven by `--project PATH`. Backend mode uses only
`waypoints_backend::Client`: it lists workflows, loads graphs, saves assignments
with a checked baseline, starts/continues/cancels runs, polls bounded ACP
events, replies to single-use permissions, reviews exact current/run/revision
patches through Rune Kit's diff view, browses pinned run/native history, and
shows snapshot and continuation state. It does not open the database. Rune-only
appearance/reduced-motion preferences live in a separate file. Native startup
and 120-frame traces pass on the development host; the user confirmed the native
macOS interaction, menu, IME/VoiceOver, close/reopen, and multi-display checks.

Run commands from the repository root. Rune is the canonical `waypoints-app`
package; the former GPUI validation commands were removed with the cutover.

## Baseline checks

```sh
cargo check -p waypoints-app --locked
cargo test -p waypoints-app --locked
cargo test --workspace --locked
cargo fmt -p waypoints-app -- --check
cargo clippy -p waypoints-app --all-targets --all-features --locked --no-deps -- -D warnings
cargo doc -p waypoints-app --no-deps --locked
cargo check --workspace --locked
```

Workspace-wide `cargo fmt --all` and Clippy still encounter pre-existing
backend/core formatting and MSRV findings; the canonical app's strict checks
are the release gate.

## Rune configuration and prerequisites

Rune and Rune Kit are pinned to
`a8ca769b944598e179f33cc6843629b90e6ca831`. Rune defaults are disabled;
`accessibility`, `dbus`, `embedded-fonts`, `metal-backend`, and
`native-file-dialog` are enabled.
This keeps native accessibility, Linux theme integration, embedded fallback
fonts, Apple's native renderer, and the native project folder picker, without
optional Ogg audio/SVG support.
Rune Kit enables only its `markdown` feature. The former GPUI dependency and
vendor patches were removed from the workspace.

Tested build/headless host: Apple silicon, macOS Darwin 25.6.0, Rust/Cargo 1.99.1
(matching Rune's pinned toolchain). The existing Apple development toolchain
was sufficient; no system packages were installed.

Linux builds need `libxkbcommon` and `libwayland-client` development libraries;
windowed X11 runs also need `libxkbcommon-x11`, and GPU rendering needs a usable
driver (software Vulkan is an upstream option). Consult the pinned core README
for distro-specific commands. Linux and Windows have not been tested here.

Upstream exposes hot reload through `rune/hot` and its `scripts/rune-dev`
external-app launcher. It has not been enabled or validated in Waypoints;
`./dev` now launches the canonical Rune app with Cargo.

## Verified Rune commands

```sh
cargo check -p waypoints-app --locked
cargo test -p waypoints-app --locked
cargo build -p waypoints-app --locked
cargo fmt -p waypoints-app -- --check
cargo clippy -p waypoints-app --all-targets --all-features --locked --no-deps -- -D warnings
cargo doc -p waypoints-app --no-deps --locked

mkdir -p target/port-info/rune
target/debug/waypoints-app \
  --snap target/port-info/rune/canvas-light.png \
  --tree target/port-info/rune/canvas-light.json --theme light --scale 2 --size 1280x820
target/debug/waypoints-app \
  --snap target/port-info/rune/workbench-dark.png \
  --tree target/port-info/rune/workbench-dark.json --theme dark --scale 2 --size 1280x820 \
  --scene workbench --at 0.8
target/debug/waypoints-app \
  --snap target/port-info/rune/large-workbench-light.png \
  --tree target/port-info/rune/large-workbench-light.json --theme light --scale 2 --size 1280x820 \
  --scene large-workbench --at 1.4
target/debug/waypoints-app \
  --script crates/waypoints-app/fixtures/canvas-tour.json \
  --snap target/port-info/rune/canvas-tour-light.png \
  --tree target/port-info/rune/canvas-tour-light.json \
  --trace target/port-info/rune/canvas-tour-light.jsonl \
  --theme light --scale 2 --size 1280x820 --at mark:end
```

The PNGs were visually inspected. Trees were checked for camera-scaled card
rects, the screen-space `workbench` dialog, seven streamed output lines after
the scripted tour, and node/edge keys. Thirty Rust tests cover the same
interactions without needing a display, including real-backend and UI-state tests:

```sh
cargo test -p waypoints-app --locked backend_
```

Those tests start an in-process backend on a temporary database/socket, create
a jj project/workflow/workspace, and use the deterministic ACP adapter at
`crates/waypoints-cli/tests/fixtures/acp_agent.py`. They cover workflow graph
loading, stale replies, draft preservation, checked assignment saves, a pending
permission, node switching while live output is pending, run completion with a
saved snapshot, and cancellation. No real model or installed ACP agent is used.


Backend-mode headless captures use an isolated temp `XDG_STATE_HOME` and a
disposable jj project. `--project`, `--workflow`, and `--agent` select the
backend; without `--project` the app is the offline fixture. Artifacts are
`target/port-info/rune/backend-light.png|json`,
`backend-tour-light.png|json|jsonl`, `backend-run-{open,start,complete,continued,cancelled}.png|json`.
The integration tests are the reproducible path; the captures record the
permission, completion, continuation, and cancellation surfaces visually.

Structured output has display-only scenes shared by headless and native QA:

```sh
target/debug/waypoints-app \
  --snap target/port-info/rune/structured-output-light.png \
  --tree target/port-info/rune/structured-output-light.json \
  --theme light --scale 2 --size 1280x820 --scene structured-output --at 0.8
target/debug/waypoints-app \
  --snap target/port-info/rune/structured-output-expanded-light.png \
  --tree target/port-info/rune/structured-output-expanded-light.json \
  --theme light --scale 2 --size 1280x820 --scene structured-output-expanded --at 0.8
```

A wheel script over the output viewport records the scrolled-away tail state and
the Jump to latest affordance. Trees are checked for Markdown text, stable tool
rows/details, replay and gap notices. The user confirmed the native scroll and
workbench feel.

Diff, history, and settings display scenes:

```sh
target/debug/waypoints-app --snap target/port-info/rune/diff-review-light.png \
  --tree target/port-info/rune/diff-review-light.json --theme light --scale 2 \
  --size 1280x820 --scene diff-review --at 0.8
target/debug/waypoints-app --snap target/port-info/rune/history-runs-light.png \
  --tree target/port-info/rune/history-runs-light.json --theme light --scale 2 \
  --size 1280x820 --scene history-runs --at 0.8
target/debug/waypoints-app --snap target/port-info/rune/history-native-light.png \
  --tree target/port-info/rune/history-native-light.json --theme light --scale 2 \
  --size 1280x820 --scene history-native --at 0.8
target/debug/waypoints-app --snap target/port-info/rune/settings-light.png \
  --tree target/port-info/rune/settings-light.json --theme light --scale 2 \
  --size 1280x820 --scene settings --at 0.5
target/debug/waypoints-app --snap target/port-info/rune/settings-shortcuts-light.png \
  --tree target/port-info/rune/settings-shortcuts-light.json --theme light --scale 2 \
  --size 1280x900 --scene settings --at 0.5
target/debug/waypoints-app --snap target/port-info/rune/graph-preview-light.png \
  --tree target/port-info/rune/graph-preview-light.json --theme light --scale 2 \
  --size 1280x820 --scene graph-preview --at 0.8
WAYPOINTS_RUNE_PREFERENCES_FILE=$PWD/target/port-info/rune/headless-prefs.json \
  target/debug/waypoints-app --script crates/waypoints-app/fixtures/hover-preview.json \
  --snap target/port-info/rune/hover-preview-light.png \
  --tree target/port-info/rune/hover-preview-light.json \
  --theme light --scale 2 --size 1280x820 --at mark:end
```

`crates/waypoints-app/fixtures/hover-preview.json` moves the pointer to the
center of node 4 at 0.1 s
and marks `end` at 0.7 s, after the 260 ms hover delay. Its tree contains
`graph-preview` and `graph-preview-summary`. Headless runs skip every preference
write; compare the disposable preferences file before and after if needed.

Assignment edit/preview scenes:

```sh
target/debug/waypoints-app --snap target/port-info/rune/assignment-light.png \
  --tree target/port-info/rune/assignment-light.json --theme light --scale 2 \
  --size 1280x820 --scene assignment --at 0.8
target/debug/waypoints-app --snap target/port-info/rune/assignment-preview-light.png \
  --tree target/port-info/rune/assignment-preview-light.json --theme light --scale 2 \
  --size 1280x820 --scene assignment-preview --at 0.8
target/debug/waypoints-app --snap target/port-info/rune/canvas-traffic-light.png \
  --tree target/port-info/rune/canvas-traffic-light.json --theme dark --scale 2 \
  --size 1280x820 --at 0.5
```

The trees contain `workbench-assignment-editor`, `assignment-notice`, and the
save/discard buttons, or `workbench-assignment-preview` clipped to its compact
viewport. Expanding the workbench makes the preview viewport taller. The diff
tree starts with a 190-point `review-files` pane and a `review-file-resize`
grip. The traffic-light capture keeps the HUD at the normal left margin
(x=18) and starts it below the lights (y≥24) when headless chrome reports the
macOS traffic-light inset.

Native menu dispatch scenes (the script `menu` event fire the same item lookup
used by native menus):

```sh
WAYPOINTS_RUNE_PREFERENCES_FILE=$PWD/target/port-info/rune/headless-prefs.json \
  target/debug/waypoints-app --script crates/waypoints-app/fixtures/menu-settings.json \
  --snap target/port-info/rune/menu-settings-light.png \
  --tree target/port-info/rune/menu-settings-light.json \
  --theme light --scale 2 --size 1280x900 --at mark:end
WAYPOINTS_RUNE_PREFERENCES_FILE=$PWD/target/port-info/rune/headless-prefs.json \
  target/debug/waypoints-app --scene project-sidebar \
  --script crates/waypoints-app/fixtures/menu-sidebar.json \
  --snap target/port-info/rune/menu-sidebar-light.png \
  --tree target/port-info/rune/menu-sidebar-light.json \
  --theme light --scale 2 --size 1280x820 --at mark:end
WAYPOINTS_RUNE_PREFERENCES_FILE=$PWD/target/port-info/rune/headless-prefs.json \
  target/debug/waypoints-app --scene project-sidebar \
  --script crates/waypoints-app/fixtures/menu-refresh.json \
  --snap target/port-info/rune/menu-refresh-light.png \
  --tree target/port-info/rune/menu-refresh-light.json \
  --theme light --scale 2 --size 1280x820 --at mark:end
```

Trees contain `settings-panel`, `project-toggle` with no `project-panel`, and
`workflows-error`/`workflows-retry` respectively. The user confirmed the native
macOS menu bar, checked/enabled states, and key equivalents.

Trees are checked for review target/file/diff keys, history rows and review
handoff buttons, and settings appearance/motion controls. The test suite covers
review parsing, mode/wrap toggles, file selection, staleness, history older-page
retention, the Settings overlay and shortcut sheet, interface-zoom
layout/geometry, pointer-hover and Return/Shift+Return previews,
tab/history/output/sidebar/add-project shortcuts, focus return, window-geometry
validation/throttled persistence, and project add/remove with a scripted folder
dialog and persisted preference file. The user exercised the native folder
picker and project add/remove. Headless `--snap`, `--tree`, and `--script` runs
skip preference writes entirely. Native runs still persist preferences, so point
`WAYPOINTS_RUNE_PREFERENCES_FILE` at an absolute disposable file for native QA.
Native macOS lifecycle is user-confirmed; Linux/Windows remain unverified. See
[readiness.md](readiness.md).

## Native validation record

For interactive use:

```sh
cargo run -p waypoints-app --locked
```

The bounded native submission and geometry check is:

```sh
rm -f target/port-info/rune/native-geometry-final.json
WAYPOINTS_RUNE_PREFERENCES_FILE=$PWD/target/port-info/rune/native-geometry-final.json \
  target/debug/waypoints-app --native \
  --trace target/port-info/rune/native-canvas-hover-geometry-final.jsonl \
  --frames 120 --theme dark
```

It completes with 120 frames, 119 presented, 1 skipped, interval p50/p95
16.67/16.67 ms in the latest run; two intermediate runs reported 6–7 skipped
with 0 failed acquires, so this host's pacing varies. No `native menu` errors are
reported on stderr. After the run
the disposable preferences file contains
`"window_geometry": {"x": 80.0, "y": 44.0, "width": 1280.0, "height": 820.0,
"scale": 2.0}`, and the trace header reports 1280×820 at scale 2.

For display clamping, seed the disposable file with
`{"x":1300,"y":100,"width":1100,"height":700,"scale":2}`, run 40 frames, and the
trace header reports 1100×700 while the saved geometry becomes
`{"x":340,"y":100,...}`. Seed `{"x":30000,"y":30000,...}` instead and the app
falls back to a centered 1280×820 window. Do not pass `--size` during these
checks, because it overrides the restored size. Earlier runs completed with the same pacing; a backend-mode trace
completed with 118 presented, 2 skipped. One post-sidebar attempt failed with
`display link creation failed: -6661` and was stopped after the tool timeout; no
app process remained. Immediate retries and the latest trace passed, so the
failure is intermittent on this host. The user confirmed native pointer/trackpad
feel, IME, VoiceOver/accessibility, close/reopen, and multi-display geometry.
Linux/Windows and real-agent execution remain unverified; do not change render
backends silently to hide a future failure.

For implementation steps, run formatting checks, relevant package tests, and
Clippy with warnings denied. Run broader workspace tests when shared code changes.
Document unsupported feature combinations rather than substituting different
checks without saying so.

## Isolation

- Fixture mode must not contact the backend or launch agents.
- Rune UI preferences must not overwrite the GPUI preference file.
- Use `WAYPOINTS_RUNE_PREFERENCES_FILE` for an isolated absolute Rune
  preferences path in native QA. Headless runs skip preference writes.
- Use disposable project/state directories and the backend's documented config
  for mutating integration checks. See [backend usage](../11-backend-cli.md).
- Rune starts the backend when a project is selected by `--project` or saved
  preferences; fixture mode and display-only scenes do not start it.
- Do not run real ACP agents or code-changing operations on a user's normal
  projects as an incidental smoke test.

## What headless checks cannot establish

Verify native input feel, pointer-anchored zoom, focus, clipboard, text selection,
IME composition, accessibility, window resize/close/reopen, and reduced motion
on actual hosts. Headless checks remain useful for deterministic application
state, geometry, rendering, and scripted interaction, but do not replace these.

Use both light/dark appearance, larger interface text, and narrow windows.
Check camera motion while output streams and a workbench is open.

## Performance evidence

Use optimized builds, the same fixture/data and comparable resolutions/scales.
Record hardware, OS, framework revision, build profile, graph/transcript size,
and accessibility state. Separate CPU frame time, GPU time, input latency,
idle wakeups, and memory rather than reducing all of them to “FPS.”

Record p50/p95 where measured, including cold/mounting versus settled behavior.
No performance threshold is established, and the former GPUI frontend is no
longer available for a side-by-side comparison. Upstream demo numbers are
background information, not Waypoints acceptance evidence.

## Handoff evidence format

For each step, record in [progress.md](progress.md):

- Outcome and changed paths.
- Exact commands run, pass/fail, and useful artifact locations.
- Native checks performed and checks not performed.
- Remaining assumptions/blockers and next concrete task.

Do not commit generated captures or large benchmark artifacts by default.
Use persistent workspace-relative artifact locations for handoffs, not per-turn
scratch paths that disappear.
