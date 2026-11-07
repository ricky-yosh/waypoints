//! Run and native-revision history with pinned pagination and review handoff.

use rune::{Color, Cursor, Drag, El, ScrollSpec, Tx, col, el, row, text};
use rune_kit::{Button, ButtonVariant, Kit, SegmentedControl};
use waypoints_core::run::SnapshotState;

use crate::state::HistoryKind;
use crate::{LoadState, State};

const LIST_MIN_WIDTH: f32 = 160.0;
const LIST_MAX_WIDTH: f32 = 360.0;
const DETAILS_MIN_WIDTH: f32 = 200.0;
const PANE_GAP: f32 = 9.0;

pub fn view(s: &State, k: &Kit, available_width: f32) -> El {
    let history = &s.review.history;
    let selected = match history.kind {
        HistoryKind::Runs => 0,
        HistoryKind::Revisions => 1,
    };
    let mut column = col().w_full().grow(1.0).min_h(0.0).gap(7.0).child(
        row()
            .w_full()
            .items_center()
            .gap(8.0)
            .flex_wrap()
            .child(
                SegmentedControl::new(&["Runs", "Native"], Some(selected))
                    .label("History kind")
                    .width(Some(238.0))
                    .on_pick(|s: &mut State, index, _| {
                        s.set_history_kind(if index == 0 {
                            HistoryKind::Runs
                        } else {
                            HistoryKind::Revisions
                        });
                    })
                    .build(k)
                    .key("history-kind"),
            )
            .child(
                Button::new("Refresh")
                    .variant(ButtonVariant::GhostMuted)
                    .help("Reload this history from its newest entry")
                    .on_press(|s: &mut State| match s.review.history.kind {
                        HistoryKind::Runs => s.load_history_runs(true),
                        HistoryKind::Revisions => s.load_history_revisions(true),
                    })
                    .build(k)
                    .key("history-refresh"),
            )
            .child(
                Button::new("Load older")
                    .variant(ButtonVariant::GhostMuted)
                    .disabled(match history.kind {
                        HistoryKind::Runs => history.runs_cursor.is_none(),
                        HistoryKind::Revisions => history.revisions_cursor.is_none(),
                    })
                    .on_press(|s: &mut State| match s.review.history.kind {
                        HistoryKind::Runs => s.load_history_runs(false),
                        HistoryKind::Revisions => s.load_history_revisions(false),
                    })
                    .build(k)
                    .key("history-load-older"),
            )
            .child(
                text(match history.kind {
                    HistoryKind::Runs => "Saved runs",
                    HistoryKind::Revisions => "Native revisions at a pinned operation",
                })
                .font(k.ty.small)
                .color(k.c.text_faint)
                .grow(1.0)
                .truncate(),
            ),
    );

    if let Some(notice) = &history.notice {
        column = column.child(
            text(notice.clone())
                .key("history-notice")
                .font(k.ty.small)
                .color(if history.notice_error {
                    k.c.danger.fg
                } else {
                    k.c.text_muted
                })
                .help(notice.clone())
                .w_full()
                .wrap()
                .max_lines(2),
        );
    }
    if history.fresh {
        column = column.child(
            row()
                .key("history-freshness")
                .w_full()
                .items_center()
                .gap(7.0)
                .child(
                    text(match history.kind {
                        HistoryKind::Runs => "A newer run was recorded.",
                        HistoryKind::Revisions => "A newer native revision was recorded.",
                    })
                    .font(k.ty.small)
                    .color(k.c.warning.fg)
                    .grow(1.0),
                )
                .child(
                    Button::new("Load newest")
                        .variant(ButtonVariant::GhostMuted)
                        .on_press(|s: &mut State| match s.review.history.kind {
                            HistoryKind::Runs => s.load_history_runs(true),
                            HistoryKind::Revisions => s.load_history_revisions(true),
                        })
                        .build(k)
                        .key("history-refresh-new"),
                ),
        );
    }

    match history.kind {
        HistoryKind::Runs => column = column.child(runs_history(s, k, available_width)),
        HistoryKind::Revisions => column = column.child(revisions_history(s, k, available_width)),
    }
    column
}

fn runs_history(s: &State, k: &Kit, available_width: f32) -> El {
    let history = &s.review.history;
    if let LoadState::Failed(error) = &history.runs_state
        && history.runs.is_empty()
    {
        return error_state(
            "history-runs-error",
            error,
            "history-runs-retry",
            k,
            |s: &mut State| s.load_history_runs(true),
        );
    }
    if history.runs.is_empty() {
        return text(if history.runs_state == LoadState::Loading {
            "Loading runs…"
        } else {
            "This node has no saved runs."
        })
        .key("history-runs-empty")
        .font(k.ty.small)
        .color(k.c.text_muted);
    }
    let selected = history.selected_run;
    let list = col()
        .key("history-runs-list")
        .min_h(0.0)
        .padding(5.0)
        .gap(2.0)
        .radius(k.radius.md)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_base)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .children(history.runs.iter().map(|run| {
            let run_id = run.run.id;
            let (label, color) = crate::workbench::run_state_label(&run.run.state, k);
            let on = selected == Some(run_id);
            el().key(format!("history-run-{run_id}"))
                .w_full()
                .padding(6.0)
                .gap(5.0)
                .radius(k.radius.sm)
                .fill(if on { k.c.layer_02 } else { Color::TRANSPARENT })
                .hover(k.c.hover)
                .cursor(Cursor::Pointer)
                .focusable(true)
                .role("button")
                .label(format!("Run #{run_id}, {label}"))
                .on_press_tx(move |s: &mut State, tx| {
                    s.review.history.selected_run = Some(run_id);
                    tx.scroll_to_instant("history-run-details", 0.0, 0.0);
                })
                .child(text(format!("#{run_id}")).font(k.ty.mono).color(if on {
                    k.c.text_base
                } else {
                    k.c.text_muted
                }))
                .child(text(label).font(k.ty.small).color(color).grow(1.0))
                .children(
                    matches!(run.run.snapshot, SnapshotState::Saved { .. })
                        .then(|| text("snapshot").font(k.ty.small).color(k.c.success.fg)),
                )
        }));

    let details = history
        .runs
        .iter()
        .find(|run| Some(run.run.id) == selected)
        .map(|run| run_details(run, k))
        .unwrap_or_else(|| {
            text("Select a run to inspect its details.")
                .font(k.ty.small)
                .color(k.c.text_faint)
        });
    split(s, k, available_width, list, details, "history-run-details")
}

fn run_details(run: &waypoints_backend::RunStatus, k: &Kit) -> El {
    let (label, color) = crate::workbench::run_state_label(&run.run.state, k);
    let saved = matches!(run.run.snapshot, SnapshotState::Saved { .. });
    col()
        .key("history-run-detail")
        .w_full()
        .gap(5.0)
        .child(
            row()
                .gap(7.0)
                .items_center()
                .child(
                    text(format!("Run #{}", run.run.id))
                        .font(k.ty.ui_medium)
                        .color(k.c.text_base),
                )
                .child(text(label).font(k.ty.small).color(color)),
        )
        .children(saved.then(|| {
            Button::new("Review saved snapshot")
                .variant(ButtonVariant::Contrast)
                .on_press(|s: &mut State| s.review_history_run())
                .build(k)
                .key("history-review-run")
        }))
        .child(
            text(format!("Prompt · {}", first_line(&run.run.prompt)))
                .font(k.ty.small)
                .color(k.c.text_muted)
                .wrap()
                .max_lines(4),
        )
        .child(
            text(crate::workbench::snapshot_label(run))
                .key("history-run-snapshot")
                .font(k.ty.small)
                .color(if saved {
                    k.c.success.fg
                } else {
                    k.c.text_faint
                })
                .wrap(),
        )
        .child(
            text(match &run.run.session_id {
                Some(session) => format!("Session · {session}"),
                None => "No saved session".to_owned(),
            })
            .font(k.ty.mono)
            .color(k.c.text_faint)
            .wrap(),
        )
        .children(run.run.continued_from.map(|source| {
            text(format!("Continued from run #{source}"))
                .font(k.ty.small)
                .color(k.c.text_faint)
        }))
}

fn revisions_history(s: &State, k: &Kit, available_width: f32) -> El {
    let history = &s.review.history;
    if let LoadState::Failed(error) = &history.revisions_state
        && history.revisions.is_empty()
    {
        return error_state(
            "history-revisions-error",
            error,
            "history-revisions-retry",
            k,
            |s: &mut State| s.load_history_revisions(true),
        );
    }
    if history.revisions.is_empty() {
        return text(if history.revisions_state == LoadState::Loading {
            "Loading native revisions…"
        } else {
            "This node has no native revisions."
        })
        .key("history-revisions-empty")
        .font(k.ty.small)
        .color(k.c.text_muted);
    }
    let selected = history.selected_revision.as_deref();
    let list = col()
        .key("history-revisions-list")
        .min_h(0.0)
        .padding(5.0)
        .gap(2.0)
        .radius(k.radius.md)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_base)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .children(history.revisions.iter().map(|entry| {
            let commit = entry.evolution.revision.commit_id.clone();
            let short = short_id(&commit);
            let on = selected == Some(commit.as_str());
            el().key(format!("history-revision-{short}"))
                .w_full()
                .padding(6.0)
                .gap(4.0)
                .radius(k.radius.sm)
                .fill(if on { k.c.layer_02 } else { Color::TRANSPARENT })
                .hover(k.c.hover)
                .cursor(Cursor::Pointer)
                .focusable(true)
                .role("button")
                .label(format!(
                    "{short}, {}{}",
                    operation_label(entry),
                    if entry.is_current {
                        ", current head"
                    } else {
                        ""
                    }
                ))
                .on_press_tx(move |s: &mut State, tx| {
                    s.review.history.selected_revision = Some(commit.clone());
                    tx.scroll_to_instant("history-revision-details", 0.0, 0.0);
                })
                .child(
                    row()
                        .w_full()
                        .gap(5.0)
                        .items_center()
                        .child(text(short).font(k.ty.mono).color(if on {
                            k.c.text_base
                        } else {
                            k.c.text_muted
                        }))
                        .children(
                            entry
                                .is_current
                                .then(|| text("head").font(k.ty.small).color(k.c.success.fg)),
                        )
                        .children((!entry.run_ids.is_empty()).then(|| {
                            text(format!("{} runs", entry.run_ids.len()))
                                .font(k.ty.small)
                                .color(k.c.text_faint)
                        })),
                )
                .child(
                    text(operation_label(entry))
                        .font(k.ty.small)
                        .color(k.c.text_faint)
                        .truncate(),
                )
        }));

    let details = history
        .revisions
        .iter()
        .find(|entry| Some(entry.evolution.revision.commit_id.as_str()) == selected)
        .map(|entry| revision_details(entry, k, s.copy_notice()))
        .unwrap_or_else(|| {
            text("Select a native revision to inspect its details.")
                .font(k.ty.small)
                .color(k.c.text_faint)
        });
    split(
        s,
        k,
        available_width,
        list,
        details,
        "history-revision-details",
    )
}

fn split(
    s: &State,
    k: &Kit,
    available_width: f32,
    list: El,
    details: El,
    details_key: &'static str,
) -> El {
    let (min, max) = crate::split::bounds(
        available_width,
        LIST_MIN_WIDTH,
        LIST_MAX_WIDTH,
        DETAILS_MIN_WIDTH,
        PANE_GAP,
    );
    let width = s.review.history.list_width.clamp(min, max);
    row()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .gap(PANE_GAP)
        .child(list.w(width).h_full().min_h(0.0))
        .child(crate::split::divider(
            "history-resize",
            "Resize history list",
            "Drag to resize the history list",
            min,
            max,
            resize_list,
            k,
        ))
        .child(
            col()
                .key(details_key)
                .grow(1.0)
                .min_h(0.0)
                .min_w(DETAILS_MIN_WIDTH)
                .padding(8.0)
                .radius(k.radius.md)
                .fill(k.c.layer_01)
                .border(1.0, k.c.border_base)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(details),
        )
}

fn resize_list(s: &mut State, drag: &mut Drag, tx: &mut Tx, min: f32, max: f32) {
    let current = s.review.history.list_width;
    crate::split::resize(
        drag,
        &mut s.review.history.list_resize_start,
        &mut s.review.history.list_width,
        current,
        min,
        max,
        tx,
    );
}

fn revision_details(
    entry: &waypoints_core::storage::NodeRevisionEntry,
    k: &Kit,
    copy_notice: Option<&str>,
) -> El {
    let revision = &entry.evolution.revision;
    let actions = row()
        .w_full()
        .gap(8.0)
        .items_center()
        .flex_wrap()
        .child(
            Button::new("Copy commit")
                .variant(ButtonVariant::GhostMuted)
                .help("Copy the full native commit ID")
                .on_press_tx(|s: &mut State, tx| {
                    if let Some(entry) = s.review.history.revisions.iter().find(|entry| {
                        Some(entry.evolution.revision.commit_id.as_str())
                            == s.review.history.selected_revision.as_deref()
                    }) {
                        tx.copy(entry.evolution.revision.commit_id.clone());
                        s.note_copied("Copied commit");
                        tx.wake_after(std::time::Duration::from_millis(2600));
                    }
                })
                .build(k)
                .key("history-copy-commit"),
        )
        .child(
            Button::new("Review this revision")
                .variant(ButtonVariant::Contrast)
                .on_press(|s: &mut State| s.review_history_revision())
                .build(k)
                .key("history-review-revision"),
        )
        .children(copy_notice.map(|notice| {
            text(notice)
                .key("history-copy-status")
                .font(k.ty.small)
                .color(k.c.success.fg)
                .role("status")
        }));
    col()
        .key("history-revision-detail")
        .w_full()
        .gap(5.0)
        .child(
            row()
                .gap(7.0)
                .items_center()
                .child(
                    text(short_id(&revision.commit_id))
                        .font(k.ty.mono)
                        .color(k.c.text_base),
                )
                .children(
                    entry
                        .is_current
                        .then(|| text("Current head").font(k.ty.small).color(k.c.success.fg)),
                )
                .children(
                    revision
                        .conflicted
                        .then(|| text("Conflicted").font(k.ty.small).color(k.c.danger.fg)),
                ),
        )
        .child(actions)
        .child(
            text(revision.description.clone())
                .font(k.ty.small)
                .color(k.c.text_muted)
                .w_full()
                .wrap()
                .max_lines(6),
        )
        .child(
            text(format!(
                "Operation · {}",
                entry.evolution.operation_description
            ))
            .font(k.ty.small)
            .color(k.c.text_faint)
            .wrap(),
        )
        .child(
            text(entry.evolution.timestamp.clone())
                .font(k.ty.mono)
                .color(k.c.text_faint),
        )
        .child(
            text(format!("Commit · {}", revision.commit_id))
                .font(k.ty.mono)
                .color(k.c.text_faint)
                .wrap(),
        )
        .children((!entry.run_ids.is_empty()).then(|| {
            text(format!(
                "Recorded by run {}",
                entry
                    .run_ids
                    .iter()
                    .map(|id| format!("#{id}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
            .font(k.ty.small)
            .color(k.c.text_muted)
        }))
}

fn error_state(
    key: &'static str,
    error: &str,
    button_key: &'static str,
    k: &Kit,
    retry: fn(&mut State),
) -> El {
    col()
        .key(key)
        .w_full()
        .gap(5.0)
        .child(
            text(error.to_owned())
                .font(k.ty.small)
                .color(k.c.danger.fg)
                .help(error.to_owned())
                .w_full()
                .wrap()
                .max_lines(4),
        )
        .child(
            Button::new("Retry")
                .variant(ButtonVariant::GhostMuted)
                .on_press(retry)
                .build(k)
                .key(button_key),
        )
}

fn operation_label(entry: &waypoints_core::storage::NodeRevisionEntry) -> String {
    let label = entry.evolution.operation_description.trim();
    if label.is_empty() {
        "Native revision".to_owned()
    } else {
        label.to_owned()
    }
}

fn first_line(text: &str) -> String {
    text.lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("No prompt")
        .trim()
        .to_owned()
}

fn short_id(commit_id: &str) -> String {
    commit_id.chars().take(12).collect()
}
