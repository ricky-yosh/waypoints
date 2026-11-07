//! Shared horizontal split-pane divider and width bounds.
//!
//! History and Changes use the same visual and pointer behavior so resizing
//! feels consistent and narrow workbenches keep both panes usable.

use rune::{Cursor, Drag, El, Phase, Tx, el};
use rune_kit::Kit;

/// Width of the draggable divider hit area.
pub(crate) const DIVIDER_WIDTH: f32 = 7.0;

/// First-pane width bounds for a split inside `available_width`.
///
/// `first_max` caps the preferred width on large windows while `second_min`
/// guarantees the details/diff pane stays readable on narrow ones.
pub(crate) fn bounds(
    available_width: f32,
    first_min: f32,
    first_max: f32,
    second_min: f32,
    gap: f32,
) -> (f32, f32) {
    let max = (available_width - second_min - DIVIDER_WIDTH - gap).clamp(first_min, first_max);
    (first_min.min(max), max)
}

/// A separator with the established drag, hover, cursor, and accessibility
/// contract. `resize` receives the clamped bounds for this layout.
pub(crate) fn divider<S: 'static>(
    key: &'static str,
    label: &'static str,
    help: &'static str,
    min: f32,
    max: f32,
    resize: fn(&mut S, &mut Drag, &mut Tx, f32, f32),
    k: &Kit,
) -> El {
    el().key(key)
        .w(DIVIDER_WIDTH)
        .h_full()
        .cursor(Cursor::ResizeColumn)
        .hover(k.c.hover)
        .role("separator")
        .label(label)
        .help(help)
        .on_drag(move |s: &mut S, drag, tx| resize(s, drag, tx, min, max))
        .child(el().w(1.0).h_full().self_center().fill(k.c.border_base))
}

/// Shared drag state update: begin from the shown width, then clamp movement.
pub(crate) fn resize(
    drag: &Drag,
    start: &mut Option<f32>,
    width: &mut f32,
    current: f32,
    min: f32,
    max: f32,
    tx: &mut Tx,
) {
    match drag.phase {
        Phase::Down => *start = Some(current.clamp(min, max)),
        Phase::Move | Phase::Up => {
            if let Some(start) = *start {
                *width = (start + drag.dx).clamp(min, max);
                tx.instant();
            }
            if drag.phase == Phase::Up {
                *start = None;
            }
        }
    }
}
