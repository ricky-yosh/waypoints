# Technical debt and follow-ups

Small, known tradeoffs that are deliberate for now. This is not a roadmap or a
priority order.

## Icon assets

The app currently embeds the complete Lucide catalog through
`gpui_kit::assets::AllAssets` (about 1,830 SVGs, roughly 7 MB of source assets)
because the icon set is still settling and the full catalog keeps iteration
simple. Once the final icon vocabulary is known, replace it with a selected
`icon_assets!` bundle composed with the default component assets. Update the
headless screenshot harness to use the same asset source at that point.

- Code: `crates/waypoints-app/src/lib.rs`, `crates/waypoints-app/src/screenshots.rs`
- Related UI notes: [Working in the desktop app](05-ui-and-diffs.md)

## Headless screenshots

The screenshot harness renders the real workspace views with deterministic
fixture data through GPUI Fast's headless renderer. It does not replace native
screen capture or real end-to-end agent validation.
