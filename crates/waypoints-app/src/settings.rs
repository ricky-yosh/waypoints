//! Rune settings overlay: appearance and reduced motion.

use rune::{Color, El, ScrollSpec, col, el, row, text};
use rune_kit::{IconButton, Kit, SegmentedControl, Switch, glyphs};

use crate::State;
use crate::preferences::Appearance;

pub fn view(s: &State, k: &Kit, size: [f32; 2]) -> El {
    let max_height = (size[1] - 32.0).max(240.0);
    let body_max = (max_height - 73.0).max(160.0);
    let mut body = col()
        .key("settings-scroll")
        .w_full()
        .max_h(body_max)
        .min_h(0.0)
        .shrink()
        .gap(13.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(
            col()
                .gap(6.0)
                .child(
                    text("Appearance")
                        .font(k.ty.small_medium)
                        .color(k.c.text_faint),
                )
                .child(
                    SegmentedControl::new(
                        &["System", "Light", "Dark"],
                        Some(s.preferences.appearance.index()),
                    )
                    .label("Appearance")
                    .width(None)
                    .on_pick(|s: &mut State, index, _| {
                        s.set_appearance(Appearance::from_index(index));
                    })
                    .build(k)
                    .key("settings-appearance"),
                ),
        )
        .child(
            col()
                .gap(6.0)
                .child(
                    text("Interface zoom")
                        .font(k.ty.small_medium)
                        .color(k.c.text_faint),
                )
                .child(
                    SegmentedControl::new(&["85%", "100%", "115%", "130%"], Some(zoom_index(s)))
                        .label("Interface zoom")
                        .width(None)
                        .on_pick(|s: &mut State, index, _| {
                            s.set_interface_zoom(
                                crate::preferences::INTERFACE_ZOOM_LEVELS[index.min(3)],
                            );
                        })
                        .build(k)
                        .key("settings-zoom"),
                ),
        )
        .child(
            row()
                .w_full()
                .items_center()
                .justify_between()
                .gap(10.0)
                .child(
                    col()
                        .gap(2.0)
                        .child(
                            text("Reduce motion")
                                .font(k.ty.small_medium)
                                .color(k.c.text_base),
                        )
                        .child(
                            text("Snap spatial motion while keeping fades.")
                                .font(k.ty.small)
                                .color(k.c.text_faint),
                        ),
                )
                .child(
                    Switch::new(s.preferences.reduced_motion)
                        .label("Reduce motion")
                        .hide_label()
                        .on_toggle(|s: &mut State| {
                            let reduced = !s.preferences.reduced_motion;
                            s.set_reduced_motion(reduced);
                        })
                        .build(k)
                        .key("settings-reduced-motion"),
                ),
        );

    let mut shortcuts = col().key("settings-shortcuts").w_full().gap(3.0).child(
        text("Keyboard shortcuts")
            .font(k.ty.small_medium)
            .color(k.c.text_faint),
    );
    for (keys, label) in [
        ("⌘,", "Open Settings"),
        ("Esc", "Close panel/workbench or clear diff selection"),
        ("Return", "Open the focused graph card"),
        ("⇧Return", "Toggle the focused graph card preview"),
        ("⌘1–4", "Workbench: Assignment, Output, Changes, History"),
        ("R / N", "History: runs or native revisions"),
        ("⌘B", "Show or hide the project navigator"),
        ("⌘O", "Add a project folder"),
        ("J / K · ] / [", "Diff: next/previous hunk or file"),
        ("⌘J", "Output: jump to latest"),
        ("⌘R", "Refresh the current surface"),
        ("⌘0 · ⌘+ · ⌘−", "Reset or zoom the canvas"),
    ] {
        shortcuts = shortcuts.child(
            row()
                .w_full()
                .gap(8.0)
                .items_center()
                .child(text(keys).font(k.ty.mono).color(k.c.text_muted).w(110.0))
                .child(text(label).font(k.ty.small).color(k.c.text_faint).grow(1.0)),
        );
    }
    body = body.child(shortcuts);

    if let Some(error) = &s.preferences_error {
        body = body.child(
            text(error.clone())
                .key("settings-error")
                .font(k.ty.small)
                .color(k.c.danger.fg)
                .w_full()
                .wrap()
                .max_lines(4),
        );
    }
    body = body.child(
        text(
            s.preferences_path
                .as_ref()
                .map_or("Preferences path unavailable".to_owned(), |path| {
                    format!("Saved to {}", path.display())
                }),
        )
        .key("settings-path")
        .font(k.ty.small)
        .color(k.c.text_faint)
        .w_full()
        .wrap()
        .max_lines(2),
    );

    let panel = col()
        .key("settings-panel")
        .w(430.0)
        .max_h(max_height)
        .padding(18.0)
        .gap(13.0)
        .radius(k.radius.xl)
        .fill(k.c.bg_base)
        .border(1.0, k.c.border_base)
        .shadow(8.0, 32.0, Color::BLACK.alpha(0.28))
        .on_press(|_: &mut State| {})
        .on_wheel(|_: &mut State, _, _| true)
        .role("dialog")
        .label("Settings")
        .focus_return("settings-open")
        .child(
            row()
                .w_full()
                .items_center()
                .justify_between()
                .child(text("Settings").font(k.ty.ui_medium).color(k.c.text_base))
                .child(
                    IconButton::new(&glyphs::CLOSE)
                        .variant(rune_kit::IconButtonVariant::GhostMuted)
                        .label("Close settings")
                        .on_press(|s: &mut State| s.settings_open = false)
                        .build(k)
                        .key("settings-close"),
                ),
        )
        .child(body);

    overlay(panel, k)
}

fn zoom_index(s: &State) -> usize {
    crate::preferences::INTERFACE_ZOOM_LEVELS
        .iter()
        .position(|zoom| (zoom - s.preferences.interface_zoom).abs() < 0.001)
        .unwrap_or(1)
}

fn overlay(panel: El, k: &Kit) -> El {
    el().key("settings-overlay")
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .center()
        .fill(k.c.scrim)
        .modal(true, |s: &mut State| s.settings_open = false)
        .child(panel)
}
