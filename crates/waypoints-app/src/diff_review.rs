//! Exact-target diff review for a node's current revision or saved run snapshot.

use rune::{Color, Cursor, Drag, El, ScrollSpec, Tx, col, el, row, text};
use rune_kit::diff::{DiffMode, DiffState, DiffView, FileStatus};
use rune_kit::{Button, ButtonVariant, Kit};

use crate::backend::ReviewTarget;
use crate::{LoadState, State};

const FILE_LIST_MIN_WIDTH: f32 = 140.0;
const FILE_LIST_MAX_WIDTH: f32 = 360.0;
const DIFF_MIN_WIDTH: f32 = 220.0;
const PANE_GAP: f32 = 6.0;

pub fn view(s: &State, k: &Kit, available_width: f32) -> El {
    let run_snapshot = s.selected_run().and_then(|run| {
        matches!(
            run.run.snapshot,
            waypoints_core::run::SnapshotState::Saved { .. }
        )
        .then_some(run.run.id)
    });
    let current = s.review.target == ReviewTarget::Current;
    let run_target = matches!(s.review.target, ReviewTarget::Run(_));

    let mut column = col().w_full().grow(1.0).min_h(0.0).gap(7.0).child(
        row()
            .w_full()
            .items_center()
            .gap(6.0)
            .flex_wrap()
            .child(
                Button::new("Current")
                    .variant(if current {
                        ButtonVariant::Contrast
                    } else {
                        ButtonVariant::GhostMuted
                    })
                    .help("Recorded native revision for this node")
                    .on_press(|s: &mut State| s.review_current())
                    .build(k)
                    .key("review-target-current"),
            )
            .child(
                Button::new(match run_snapshot {
                    Some(run_id) => format!("Run #{run_id} snapshot"),
                    None => "Run snapshot".to_owned(),
                })
                .variant(if run_target {
                    ButtonVariant::Contrast
                } else {
                    ButtonVariant::GhostMuted
                })
                .disabled(run_snapshot.is_none())
                .help("Exact revision saved by this node's latest run")
                .on_press(|s: &mut State| s.review_latest_run())
                .build(k)
                .key("review-target-run"),
            )
            .child(
                text(match &s.review.target {
                    ReviewTarget::Current => "Current recorded revision".to_owned(),
                    ReviewTarget::Run(run_id) => format!("Pinned run #{run_id}"),
                    ReviewTarget::Revision(commit) => {
                        format!("Pinned revision {}", short_id(commit))
                    }
                })
                .key("review-target-label")
                .font(k.ty.small)
                .color(k.c.text_faint)
                .grow(1.0)
                .truncate(),
            )
            .child(
                Button::new("Refresh")
                    .variant(ButtonVariant::GhostMuted)
                    .help("Reload this exact review target")
                    .on_press(|s: &mut State| s.request_review(s.review.target.clone()))
                    .build(k)
                    .key("review-refresh"),
            ),
    );

    if s.review
        .review
        .as_ref()
        .is_some_and(|review| review.revision.conflicted)
    {
        column = column.child(
            text("This native revision has unresolved conflicts; the patch preserves jj's conflict markers.")
                .key("review-conflicted")
                .font(k.ty.small)
                .color(k.c.warning.fg)
                .w_full()
                .wrap(),
        );
    }

    if s.review.stale {
        column = column.child(
            row()
                .key("review-stale")
                .w_full()
                .gap(7.0)
                .items_center()
                .child(
                    text("A newer run snapshot was recorded after this review loaded.")
                        .font(k.ty.small)
                        .color(k.c.warning.fg)
                        .grow(1.0)
                        .wrap(),
                )
                .child(
                    Button::new("Reload current")
                        .variant(ButtonVariant::GhostMuted)
                        .on_press(|s: &mut State| s.review_current())
                        .build(k)
                        .key("review-reload-current"),
                ),
        );
    }

    match &s.review.state {
        LoadState::Loading => {
            column = column.child(
                text("Loading the exact patch…")
                    .key("review-loading")
                    .font(k.ty.small)
                    .color(k.c.text_muted),
            );
        }
        LoadState::Failed(error) => {
            column = column.child(
                col()
                    .key("review-error")
                    .w_full()
                    .gap(5.0)
                    .child(
                        text(error.clone())
                            .font(k.ty.small)
                            .color(k.c.danger.fg)
                            .help(error.clone())
                            .w_full()
                            .wrap()
                            .max_lines(3),
                    )
                    .child(
                        Button::new("Retry review")
                            .variant(ButtonVariant::GhostMuted)
                            .on_press(|s: &mut State| s.request_review(s.review.target.clone()))
                            .build(k)
                            .key("review-retry"),
                    ),
            );
        }
        _ => {}
    }

    if let Some(review) = &s.review.review {
        let files = s.review.diff.files();
        let additions: usize = files.iter().map(|file| file.additions()).sum();
        let deletions: usize = files.iter().map(|file| file.deletions()).sum();
        column = column.child(
            row()
                .w_full()
                .gap(7.0)
                .items_center()
                .child(
                    text(format!("{} files", files.len()))
                        .key("review-file-count")
                        .font(k.ty.small_medium)
                        .color(k.c.text_faint),
                )
                .child(
                    text(format!("+{additions}"))
                        .font(k.ty.small)
                        .color(k.c.success.fg),
                )
                .child(
                    text(format!("−{deletions}"))
                        .font(k.ty.small)
                        .color(k.c.danger.fg),
                )
                .child(
                    text(short_id(&review.revision.commit_id))
                        .key("review-commit")
                        .font(k.ty.mono)
                        .color(k.c.text_faint)
                        .help(review.revision.commit_id.clone()),
                )
                .child(
                    Button::new(match s.review.diff.mode() {
                        DiffMode::Unified => "Split",
                        DiffMode::Split => "Unified",
                    })
                    .variant(ButtonVariant::GhostMuted)
                    .on_press(|s: &mut State| {
                        let mode = match s.review.diff.mode() {
                            DiffMode::Unified => DiffMode::Split,
                            DiffMode::Split => DiffMode::Unified,
                        };
                        s.review.diff.set_mode(mode);
                    })
                    .build(k)
                    .key("review-toggle-mode"),
                )
                .child(
                    Button::new(if s.review.diff.wrap() {
                        "Unwrap"
                    } else {
                        "Wrap"
                    })
                    .variant(ButtonVariant::GhostMuted)
                    .on_press(|s: &mut State| {
                        let wrap = !s.review.diff.wrap();
                        s.review.diff.set_wrap(wrap);
                    })
                    .build(k)
                    .key("review-toggle-wrap"),
                )
                .child(
                    Button::new("Copy commit")
                        .variant(ButtonVariant::GhostMuted)
                        .help("Copy the full commit ID")
                        .on_press_tx(|s: &mut State, tx| {
                            if let Some(review) = &s.review.review {
                                tx.copy(review.revision.commit_id.clone());
                                s.note_copied("Copied commit");
                                tx.wake_after(std::time::Duration::from_millis(2600));
                            }
                        })
                        .build(k)
                        .key("review-copy-commit"),
                )
                .children(s.copy_notice().map(|notice| {
                    text(notice)
                        .key("review-copy-status")
                        .font(k.ty.small)
                        .color(k.c.success.fg)
                        .role("status")
                })),
        );

        let (min, max) = crate::split::bounds(
            available_width,
            FILE_LIST_MIN_WIDTH,
            FILE_LIST_MAX_WIDTH,
            DIFF_MIN_WIDTH,
            PANE_GAP,
        );
        let width = s.review.file_list_width.clamp(min, max);
        if files.is_empty() {
            column = column.child(
                text("This exact target has no changed files.")
                    .key("review-empty")
                    .font(k.ty.small)
                    .color(k.c.text_muted),
            );
        } else {
            column = column.child(
                row()
                    .w_full()
                    .grow(1.0)
                    .min_h(0.0)
                    .gap(PANE_GAP)
                    .child(file_list(s, k, width))
                    .child(crate::split::divider(
                        "review-file-resize",
                        "Resize changed files",
                        "Drag to resize changed files",
                        min,
                        max,
                        resize_files,
                        k,
                    ))
                    .child(
                        col()
                            .grow(1.0)
                            .min_w(DIFF_MIN_WIDTH)
                            .min_h(0.0)
                            .key("review-diff-surface")
                            .child(
                                DiffView::new(
                                    |s: &State| -> &DiffState { &s.review.diff },
                                    |s: &mut State| -> &mut DiffState { &mut s.review.diff },
                                )
                                .key("review-diff")
                                .on_visible_file(|s: &mut State, index| {
                                    s.review.selected_file = Some(index);
                                })
                                .build(s, k),
                            ),
                    ),
            );
        }
    } else if matches!(s.review.state, LoadState::Idle) {
        column = column.child(
            text("Select a review target to load its exact patch.")
                .key("review-idle")
                .font(k.ty.small)
                .color(k.c.text_muted),
        );
    }

    column
}

fn resize_files(s: &mut State, drag: &mut Drag, tx: &mut Tx, min: f32, max: f32) {
    let current = s.review.file_list_width;
    crate::split::resize(
        drag,
        &mut s.review.file_resize_start,
        &mut s.review.file_list_width,
        current,
        min,
        max,
        tx,
    );
}

fn file_list(s: &State, k: &Kit, width: f32) -> El {
    let files = s.review.diff.files();
    col()
        .key("review-files")
        .w(width)
        .h_full()
        .min_h(0.0)
        .padding(5.0)
        .gap(2.0)
        .radius(k.radius.md)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_base)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .children(files.iter().enumerate().map(|(index, file)| {
            let selected = s.review.selected_file == Some(index);
            let (status, color) = match file.status {
                FileStatus::Modified => ("M", k.c.warning.fg),
                FileStatus::Added => ("A", k.c.success.fg),
                FileStatus::Deleted => ("D", k.c.danger.fg),
                FileStatus::Renamed => ("R", k.c.info.fg),
                FileStatus::Copied => ("C", k.c.info.fg),
            };
            el().key(format!("review-file-{index}"))
                .w_full()
                .padding(5.0)
                .gap(5.0)
                .radius(k.radius.sm)
                .fill(if selected {
                    k.c.layer_02
                } else {
                    Color::TRANSPARENT
                })
                .hover(k.c.hover)
                .cursor(Cursor::Pointer)
                .focusable(true)
                .role("button")
                .label(format!(
                    "{} {}{}",
                    status,
                    file.path(),
                    if file.binary { ", binary" } else { "" }
                ))
                .on_press_tx(move |s: &mut State, tx| s.scroll_review_to_file(index, tx))
                .child(text(status).font(k.ty.mono).color(color))
                .child(
                    text(file.path().to_owned())
                        .font(k.ty.small)
                        .color(if selected {
                            k.c.text_base
                        } else {
                            k.c.text_muted
                        })
                        .truncate_middle()
                        .grow(1.0),
                )
                .children(
                    file.binary
                        .then(|| text("bin").font(k.ty.small).color(k.c.text_faint)),
                )
        }))
}

fn short_id(commit_id: &str) -> String {
    commit_id.chars().take(12).collect()
}
