# Validation and evidence

## Current state

The `waypoints-rune` package is an isolated bootstrap fixture. It does not link
the Waypoints backend/core, access projects, or read/write UI preferences.
Native startup is user-confirmed from the interactive desktop; the automated
trace failure and remaining native checks are recorded below.

Run commands from the repository root. Existing GPUI commands below come from
the current workspace and [desktop validation docs](../05-ui-and-diffs.md#validation).

## Baseline checks

```sh
cargo check -p waypoints-app
cargo test -p waypoints-app
```

For reference fixtures (writes generated PNGs; choose a separate output directory
so existing captures are not overwritten):

```sh
cargo run -p waypoints-app --features screenshot-fixtures \
  --bin waypoints-screenshots -- target/port-info/gpui-reference
```

On a host with a native window server:

```sh
cargo run -p waypoints-app --features native-qa --bin waypoints-native-qa
```

Native fixture success ends with `NATIVE_QA_OK`. A failure to launch because no
display is available is not evidence of native correctness.

## Rune configuration and prerequisites

Rune and Rune Kit are pinned to
`a8ca769b944598e179f33cc6843629b90e6ca831`. Rune defaults are disabled;
`accessibility`, `dbus`, `embedded-fonts`, and `metal-backend` are enabled.
This keeps native accessibility, Linux theme integration, embedded fallback
fonts, and Apple's native renderer, without optional Ogg audio/SVG support.
Rune Kit has no optional features enabled yet. No GPUI dependency is introduced
into the Rune executable.

Tested build/headless host: Apple silicon, macOS Darwin 25.6.0, Rust/Cargo 1.99.1
(matching Rune's pinned toolchain). The existing Apple development toolchain
was sufficient; no system packages were installed.

Linux builds need `libxkbcommon` and `libwayland-client` development libraries;
windowed X11 runs also need `libxkbcommon-x11`, and GPU rendering needs a usable
driver (software Vulkan is an upstream option). Consult the pinned core README
for distro-specific commands. Linux and Windows have not been tested here.

Upstream exposes hot reload through `rune/hot` and its `scripts/rune-dev`
external-app launcher. It has not been enabled or validated in Waypoints;
the existing `./dev` still targets GPUI.

## Verified Rune commands

```sh
cargo check -p waypoints-rune --locked
cargo test -p waypoints-rune --locked
cargo build -p waypoints-rune --locked
cargo fmt -p waypoints-rune -- --check
cargo clippy -p waypoints-rune --all-targets --all-features --locked -- -D warnings
cargo doc -p waypoints-rune --no-deps --locked

mkdir -p target/port-info/rune
target/debug/waypoints-rune \
  --snap target/port-info/rune/bootstrap-dark.png \
  --tree target/port-info/rune/bootstrap-dark.json --theme dark --scale 2
target/debug/waypoints-rune \
  --script crates/waypoints-rune/fixtures/press.json \
  --snap target/port-info/rune/pressed-light.png \
  --tree target/port-info/rune/pressed-light.json --theme light --scale 2 --at 0.5
```

Both images were visually inspected. The element-tree entries keyed
`interaction-count` were checked for `Interactions: 0` and `Interactions: 1`,
respectively. The Rust runtime test independently presses the production Kit
button and asserts both state and rendered text. The fixture is the entire
application at this stage, so no separate fixture flag is needed.

## Pending native check

For interactive use:

```sh
cargo run -p waypoints-rune --locked
```

The bounded native submission check was attempted as:

```sh
target/debug/waypoints-rune --native \
  --trace target/port-info/rune/native.jsonl --frames 30
```

It reported `display link creation failed: -6661` and was forcibly stopped after
20 seconds. No lingering `waypoints-rune` process was found afterward.
Ricky subsequently confirmed “It works” after the interactive launch instruction.
Native startup is therefore user-confirmed, but the automated trace is not a
pass, and normal shutdown and detailed input/lifecycle behavior remain unverified.
There is no established root cause for the automated failure. Do not change
render backends silently to hide it.

For implementation steps, run formatting checks, relevant package tests, and
Clippy with warnings denied. Run broader workspace tests when shared code changes.
Document unsupported feature combinations rather than substituting different
checks without saying so.

## Isolation

- Fixture mode must not contact the backend or launch agents.
- Rune UI preferences must not overwrite the GPUI preference file.
- Existing app QA supports `WAYPOINTS_WORKSPACE_FILE` for an isolated absolute
  preferences path; inspect Rune's implementation before assuming the same hook.
- Use disposable project/state directories and the backend's documented config
  for mutating integration checks. See [backend usage](../11-backend-cli.md).
- Normal app startup does not currently start the backend automatically; preserve
  or explicitly document a changed policy.
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
No performance threshold or Rune-over-GPUI claim is established yet; agree on
budgets from the first representative measurements. Upstream demo numbers are
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
