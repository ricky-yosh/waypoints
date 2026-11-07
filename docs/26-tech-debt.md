# Technical debt and follow-ups

Small, known tradeoffs that are deliberate for now. This is not a roadmap or a
priority order.

## Rune platform boundaries

Rune's native menu API is macOS-only at the pinned revision. Linux and Windows
keep the in-app controls but have no native menu bar. Native Tab traversal, IME,
VoiceOver/accessibility, close/reopen, multi-window, and multi-display geometry
still need person checks on supported hosts. The app does not claim those as
passed from headless tests.

## Display geometry

Saved window geometry is numerically validated and clamped to a connected
display, but display hot-plug and mixed-scale multi-display arrangements still
need manual verification. The diff-pane divider is pointer-only; no keyboard
resize is implemented.

## History refresh

A full History refresh replaces the loaded view with loading/error, unlike a
failed older-page request, which retains loaded rows. This matches the recorded
behavior and is deliberate.

## Rune preferences

Rune stores its own preferences at `waypoints/rune-workspace.json` under the
platform state directory. The former GPUI workspace file is neither read nor
written. Headless render/script runs never write preferences; native runs do.
