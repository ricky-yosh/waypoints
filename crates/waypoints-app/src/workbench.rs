//! Floating, screen-space workbench for the fixture canvas.

use rune::{Color, El, ScrollSpec, col, el, row, text};
use rune_kit::{
    Button, ButtonVariant, IconButton, Kit, Markdown, SegmentedControl, TextArea, glyphs,
};

use crate::fixture::Node;
use crate::state::WorkbenchTab;
use crate::{LoadState, Mode, State};
use waypoints_core::run::{ContinuationAssessment, SnapshotState};

pub const WORKBENCH_WIDTH: f32 = 760.0;
pub const WORKBENCH_HEIGHT: f32 = 328.0;

pub fn view(s: &State, k: &Kit, window_width: f32, window_height: f32) -> El {
    let Some(id) = s.selected else {
        return el();
    };
    let Some(node) = s.graph.node(id) else {
        return el();
    };
    let width = (window_width - 32.0).clamp(320.0, WORKBENCH_WIDTH);
    let height = if s.workbench_expanded {
        (window_height - 120.0).max(WORKBENCH_HEIGHT)
    } else {
        WORKBENCH_HEIGHT
    };
    let panel = col()
        .key("workbench")
        .w(width)
        .h(height)
        .padding(18.0)
        .gap(12.0)
        .radius(k.radius.xl)
        .clip_rounded(k.radius.xl)
        .fill(k.c.bg_base)
        .border(1.0, k.c.border_base)
        .shadow(6.0, 28.0, Color::BLACK.alpha(0.22))
        .on_press(|_: &mut State| {})
        .on_wheel(|_: &mut State, _, _| true)
        .role("dialog")
        .label("Node workbench")
        .focus_return(format!("node-{id}"))
        .child(header(s, node, k));
    let panel = if s.mode == Mode::Backend {
        panel
            .child(tabs(s, k))
            .child(match s.review.tab {
                WorkbenchTab::Assignment => assignment_backend(s, node, k),
                WorkbenchTab::Output => backend_output(s, k),
                WorkbenchTab::Changes => crate::diff_review::view(s, k),
                WorkbenchTab::History => crate::history_view::view(s, k),
            })
            .child(footer(s, k))
    } else {
        panel
            .child(
                row()
                    .w_full()
                    .grow(1.0)
                    .min_h(0.0)
                    .gap(18.0)
                    .child(assignment(s, node, k))
                    .child(output(s, k)),
            )
            .child(footer(s, k))
    };

    el().pin(Some(0.0), None, Some(0.0), Some(24.0))
        .w_full()
        .justify_center()
        .child(panel)
}

fn header(s: &State, node: &Node, k: &Kit) -> El {
    let status = node.status.color(k);
    row()
        .w_full()
        .gap(12.0)
        .items_center()
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(3.0)
                .child(
                    row()
                        .gap(8.0)
                        .items_center()
                        .child(
                            text(format!("NODE #{}", node.id))
                                .font(k.ty.small_medium)
                                .color(k.c.text_faint),
                        )
                        .child(
                            row()
                                .padding(3.0)
                                .gap(5.0)
                                .items_center()
                                .radius(999.0)
                                .fill(status.alpha(0.14))
                                .child(el().size(5.0, 5.0).radius(2.5).fill(status).a11y_hidden())
                                .child(
                                    text(node.status.label())
                                        .font(k.ty.small)
                                        .color(k.c.text_base),
                                ),
                        ),
                )
                .child(
                    text(node.title.clone())
                        .key("workbench-title")
                        .font(k.ty.ui_medium)
                        .color(k.c.text_base)
                        .truncate(),
                ),
        )
        .child(
            Button::new(if s.workbench_expanded {
                "Restore"
            } else {
                "Expand"
            })
            .variant(ButtonVariant::GhostMuted)
            .help(if s.workbench_expanded {
                "Restore the floating workbench size"
            } else {
                "Expand the workbench over the workflow area"
            })
            .on_press(|s: &mut State| s.workbench_expanded = !s.workbench_expanded)
            .build(k)
            .key("workbench-expand"),
        )
        .child(
            IconButton::new(&glyphs::CLOSE)
                .variant(rune_kit::IconButtonVariant::GhostMuted)
                .label("Close workbench")
                .help("Close workbench")
                .on_press(|s: &mut State| s.workbench_open = false)
                .build(k)
                .key("workbench-close"),
        )
        .child(
            text(if s.mode == Mode::Backend {
                match s.graph_state {
                    LoadState::Loading => "Loading graph".to_owned(),
                    LoadState::Failed(_) => "Graph unavailable".to_owned(),
                    _ => "Backend assignment".to_owned(),
                }
            } else if s.streaming {
                "Streaming · presentation-only fixture".to_owned()
            } else {
                "Idle · presentation-only fixture".to_owned()
            })
            .font(k.ty.small)
            .color(if s.streaming && s.mode == Mode::Fixture {
                k.c.info.fg
            } else {
                k.c.text_faint
            }),
        )
}

fn tabs(s: &State, k: &Kit) -> El {
    let selected = match s.review.tab {
        WorkbenchTab::Assignment => 0,
        WorkbenchTab::Output => 1,
        WorkbenchTab::Changes => 2,
        WorkbenchTab::History => 3,
    };
    SegmentedControl::new(
        &["Assignment", "Output", "Changes", "History"],
        Some(selected),
    )
    .label("Workbench view")
    .width(None)
    .on_pick(|s: &mut State, index, _| {
        s.set_workbench_tab(match index {
            0 => WorkbenchTab::Assignment,
            1 => WorkbenchTab::Output,
            2 => WorkbenchTab::Changes,
            _ => WorkbenchTab::History,
        });
    })
    .build(k)
    .key("workbench-tabs")
}

fn assignment_backend(s: &State, node: &Node, k: &Kit) -> El {
    let draft = s.draft_for(node.id);
    let saving = draft.is_some_and(|draft| draft.pending.is_some());
    let dirty = draft.is_some_and(|draft| draft.dirty());
    let state = if saving {
        ("Saving…", k.c.warning.fg)
    } else if draft.is_some_and(|draft| draft.error.is_some()) {
        ("Save failed", k.c.danger.fg)
    } else if dirty {
        ("Unsaved", k.c.warning.fg)
    } else {
        ("Saved", k.c.success.fg)
    };

    let mut column = col().w_full().grow(1.0).min_h(0.0).gap(6.0).child(
        row()
            .w_full()
            .items_center()
            .justify_between()
            .child(
                row()
                    .gap(6.0)
                    .items_center()
                    .child(
                        text("Assignment")
                            .font(k.ty.small_medium)
                            .color(k.c.text_faint),
                    )
                    .child(
                        text(state.0)
                            .key("assignment-state")
                            .font(k.ty.small)
                            .color(state.1),
                    ),
            )
            .child(
                text(format!("{} deps", s.graph.incoming(node.id)))
                    .font(k.ty.small)
                    .color(k.c.text_faint),
            ),
    );

    if let Some(draft) = draft {
        column = column.child(
            SegmentedControl::new(&["Edit", "Preview"], Some(usize::from(draft.preview)))
                .label("Assignment mode")
                .width(Some(160.0))
                .on_pick(|s: &mut State, index, _| {
                    let node_id = s.selected.unwrap_or_default();
                    if let Some(draft) = s
                        .selected_workflow
                        .and_then(|workflow| s.drafts.get_mut(&(workflow, node_id)))
                    {
                        draft.preview = index == 1;
                    }
                })
                .build(k)
                .key("assignment-mode"),
        );
        if draft.preview {
            column = column.child(
                col()
                    .key("workbench-assignment-preview-scroll")
                    .w_full()
                    .grow(1.0)
                    .min_h(0.0)
                    .padding(9.0)
                    .radius(k.radius.md)
                    .fill(k.c.layer_01)
                    .border(1.0, k.c.border_base)
                    .scroll(ScrollSpec {
                        stick_bottom: false,
                    })
                    .child(
                        Markdown::new(draft.edit.text.clone())
                            .selectable(true)
                            .build(k)
                            .key("workbench-assignment-preview"),
                    ),
            );
        } else {
            column = column.child(
                col()
                    .key("workbench-assignment-editor-scroll")
                    .w_full()
                    .grow(1.0)
                    .min_h(0.0)
                    .scroll(ScrollSpec {
                        stick_bottom: false,
                    })
                    .child(
                        TextArea::new(&draft.edit)
                            .label("Node assignment")
                            .placeholder("Write the node assignment")
                            .width(None)
                            .rows(6)
                            .on_edit(|s: &mut State| s.assignment_edit_mut())
                            .on_submit(|s: &mut State, tx| {
                                s.save_assignment();
                                tx.instant();
                            })
                            .build(k)
                            .key("workbench-assignment-editor"),
                    ),
            );
        }
    } else {
        column = column.child(
            text("Waiting for the assignment to load…")
                .font(k.ty.ui)
                .color(k.c.text_muted),
        );
    }

    let save = Button::new(if saving {
        "Saving…"
    } else {
        "Save assignment"
    })
    .variant(ButtonVariant::Contrast)
    .disabled(!dirty || saving)
    .on_press(|s: &mut State| s.save_assignment())
    .build(k)
    .key("workbench-save");
    let discard = Button::new("Load saved")
        .variant(ButtonVariant::GhostMuted)
        .disabled(!dirty || saving)
        .on_press(|s: &mut State| s.discard_assignment())
        .build(k)
        .key("workbench-discard");
    column = column.child(
        row()
            .w_full()
            .gap(8.0)
            .items_center()
            .child(save)
            .child(discard),
    );

    if let Some(draft) = draft {
        if let Some(error) = &draft.error {
            column = column.child(
                text(error.clone())
                    .key("assignment-error")
                    .font(k.ty.small)
                    .color(k.c.danger.fg)
                    .w_full()
                    .wrap()
                    .max_lines(3),
            );
        } else if let Some(notice) = &draft.notice {
            column = column.child(
                text(notice.clone())
                    .key("assignment-notice")
                    .font(k.ty.small)
                    .color(k.c.text_faint)
                    .w_full()
                    .wrap()
                    .max_lines(2),
            );
        }
    }
    column
}

fn backend_output(s: &State, k: &Kit) -> El {
    let run = s.selected_run();
    let active = run.is_some_and(crate::run_is_active);
    let (state_label, state_color) = run.map_or(("No run", k.c.text_faint), |run| {
        run_state_label(&run.run.state, k)
    });
    let mut column = col().grow(1.0).min_h(0.0).min_w(0.0).gap(6.0).child(
        row()
            .w_full()
            .items_center()
            .justify_between()
            .child(
                row()
                    .gap(7.0)
                    .items_center()
                    .child(
                        text("Run output")
                            .font(k.ty.small_medium)
                            .color(k.c.text_faint),
                    )
                    .child(
                        text(state_label)
                            .key("run-state")
                            .font(k.ty.small)
                            .color(state_color),
                    ),
            )
            .child(if active {
                Button::new("Cancel run")
                    .variant(ButtonVariant::Danger)
                    .on_press(|s: &mut State| s.cancel_run())
                    .build(k)
                    .key("run-cancel")
            } else {
                let enabled = s.agent.is_some();
                Button::new("Start fresh run")
                    .variant(ButtonVariant::Neutral)
                    .disabled(!enabled)
                    .help(if enabled {
                        "Start a fresh run using the saved assignment"
                    } else {
                        "Set --agent /absolute/path/to/acp-executable"
                    })
                    .on_press(|s: &mut State| s.start_run())
                    .build(k)
                    .key("run-start")
            }),
    );

    if let Some(error) = &s.run_error {
        column = column.child(
            text(error.clone())
                .key("run-error")
                .font(k.ty.small)
                .color(k.c.danger.fg)
                .w_full()
                .wrap()
                .max_lines(3),
        );
    } else if let Some(notice) = &s.run_notice {
        column = column.child(
            text(notice.clone())
                .key("run-notice")
                .font(k.ty.small)
                .color(k.c.text_faint)
                .wrap()
                .max_lines(2),
        );
    }

    if let Some(run) = run {
        if let Some(detail) = run_state_detail(&run.run.state) {
            column = column.child(
                text(detail)
                    .font(k.ty.small)
                    .color(k.c.danger.fg)
                    .w_full()
                    .wrap()
                    .max_lines(3),
            );
        }
        column = column.child(
            text(snapshot_label(run))
                .key("run-snapshot")
                .font(k.ty.small)
                .color(k.c.text_faint)
                .w_full()
                .wrap()
                .max_lines(2),
        );
        if let Some(permission) = &run.permission {
            column = column.child(permission_card(permission, k));
        }
    }

    if s.run_log.blocks().is_empty() {
        column = column.child(
            text(if run.is_some() {
                "No structured output yet"
            } else {
                "Start a run to stream structured output"
            })
            .key("run-output-empty")
            .font(k.ty.small)
            .color(k.c.text_muted),
        );
    } else {
        column = column.child(crate::run_output::view(s, k));
    }

    if let Some(run) = run
        && !crate::run_is_active(run)
        && run.run.session_id.is_some()
        && let Some((run_id, assessment)) = &s.continuation
        && *run_id == run.run.id
    {
        match assessment {
            ContinuationAssessment::Eligible { .. } => {
                column = column.child(
                    TextArea::new(&s.follow_up)
                        .label("Follow-up instructions")
                        .placeholder("Follow-up instructions for this session")
                        .width(None)
                        .rows(2)
                        .on_edit(|s: &mut State| s.follow_up_edit_mut())
                        .on_submit(|s: &mut State, tx| {
                            s.continue_run();
                            tx.instant();
                        })
                        .build(k)
                        .key("run-follow-up"),
                );
                column = column.child(
                    Button::new("Continue session")
                        .variant(ButtonVariant::Contrast)
                        .on_press(|s: &mut State| s.continue_run())
                        .build(k)
                        .key("run-continue"),
                );
            }
            ContinuationAssessment::Blocked { detail, .. } => {
                column = column.child(
                    text(format!("Continue unavailable · {detail}"))
                        .key("run-continuation-blocked")
                        .font(k.ty.small)
                        .color(k.c.text_muted)
                        .w_full()
                        .wrap()
                        .max_lines(3),
                );
            }
            _ => {}
        }
    }

    column
}

fn permission_card(permission: &waypoints_backend::Permission, k: &Kit) -> El {
    let title = permission.tool_call["title"]
        .as_str()
        .unwrap_or("Agent permission request");
    let mut actions = row().w_full().gap(6.0).flex_wrap();
    for option in &permission.options {
        let option_id = option.option_id.clone();
        let permission_id = permission.id;
        let variant = if option.kind.starts_with("allow") {
            ButtonVariant::Contrast
        } else {
            ButtonVariant::GhostMuted
        };
        actions = actions.child(
            Button::new(option.name.clone())
                .variant(variant)
                .help(format!("Reply once with {}", option.kind))
                .on_press(move |s: &mut State| {
                    s.reply_permission(permission_id, option_id.clone());
                })
                .build(k)
                .key(format!("permission-{}", option.option_id)),
        );
    }
    col()
        .key("run-permission")
        .w_full()
        .gap(6.0)
        .padding(9.0)
        .radius(k.radius.md)
        .fill(k.c.warning.bg)
        .border(1.0, k.c.warning.border)
        .child(
            text(format!("Permission required · {title}"))
                .font(k.ty.small_medium)
                .color(k.c.warning.fg)
                .w_full()
                .wrap(),
        )
        .child(actions)
}

pub(crate) fn run_state_label(
    state: &waypoints_core::run::RunState,
    k: &Kit,
) -> (&'static str, Color) {
    match state {
        waypoints_core::run::RunState::Starting => ("Starting", k.c.warning.fg),
        waypoints_core::run::RunState::Running => ("Running", k.c.info.fg),
        waypoints_core::run::RunState::AwaitingPermission => ("Needs permission", k.c.warning.fg),
        waypoints_core::run::RunState::Cancelling => ("Cancelling", k.c.warning.fg),
        waypoints_core::run::RunState::Finalizing => ("Finalizing", k.c.info.fg),
        waypoints_core::run::RunState::Completed => ("Run completed", k.c.success.fg),
        waypoints_core::run::RunState::Cancelled => ("Cancelled", k.c.text_muted),
        waypoints_core::run::RunState::Failed { .. } => ("Run failed", k.c.danger.fg),
        waypoints_core::run::RunState::Interrupted { .. } => ("Interrupted", k.c.danger.fg),
        waypoints_core::run::RunState::Recovered { .. } => ("Recovered", k.c.text_muted),
        _ => ("Unknown", k.c.text_faint),
    }
}

fn run_state_detail(state: &waypoints_core::run::RunState) -> Option<String> {
    match state {
        waypoints_core::run::RunState::Failed { detail } => Some(format!("Run failed · {detail}")),
        waypoints_core::run::RunState::Interrupted { detail, .. } => {
            Some(format!("Interrupted · {detail}"))
        }
        waypoints_core::run::RunState::Recovered { detail, .. } => {
            Some(format!("Recovered · {detail}"))
        }
        _ => None,
    }
}

pub(crate) fn snapshot_label(run: &waypoints_backend::RunStatus) -> String {
    match &run.run.snapshot {
        SnapshotState::Saved { revision } => {
            let short: String = revision.commit_id.chars().take(12).collect();
            format!("Snapshot saved · {short}")
        }
        SnapshotState::Failed { detail } => format!("Snapshot failed · {detail}"),
        SnapshotState::NotRecorded => "Snapshot not recorded".to_owned(),
        _ => "Snapshot state unavailable".to_owned(),
    }
}

fn assignment(s: &State, node: &Node, k: &Kit) -> El {
    col()
        .w(322.0)
        .min_h(0.0)
        .gap(6.0)
        .child(
            row()
                .w_full()
                .items_center()
                .justify_between()
                .child(
                    text("Assignment")
                        .font(k.ty.small_medium)
                        .color(k.c.text_faint),
                )
                .child(
                    text(format!("{} deps", s.graph.incoming(node.id)))
                        .font(k.ty.small)
                        .color(k.c.text_faint),
                ),
        )
        .child(
            col()
                .key("workbench-assignment-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .padding(9.0)
                .radius(k.radius.md)
                .fill(k.c.layer_01)
                .border(1.0, k.c.border_base)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(
                    text(node.assignment.clone())
                        .key("workbench-assignment")
                        .font(k.ty.ui)
                        .color(k.c.text_base)
                        .w_full()
                        .wrap()
                        .selectable(true)
                        .selection_color(k.selection()),
                ),
        )
}

fn output(s: &State, k: &Kit) -> El {
    col()
        .grow(1.0)
        .min_h(0.0)
        .min_w(0.0)
        .gap(6.0)
        .child(
            row()
                .w_full()
                .items_center()
                .justify_between()
                .child(
                    text("Synthetic output")
                        .font(k.ty.small_medium)
                        .color(k.c.text_faint),
                )
                .child(
                    text(format!("{} lines", s.output.len()))
                        .key("workbench-output-count")
                        .font(k.ty.small)
                        .color(k.c.text_faint),
                ),
        )
        .child(
            col()
                .key("workbench-output-log")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .padding(9.0)
                .gap(2.0)
                .radius(k.radius.md)
                .fill(k.c.layer_01)
                .border(1.0, k.c.border_base)
                .scroll(ScrollSpec { stick_bottom: true })
                .children(s.output.iter().map(|line| {
                    row()
                        .key(format!("workbench-line-{}", line.seq))
                        .w_full()
                        .gap(7.0)
                        .items_start()
                        .child(
                            text(format!("{:02}", line.seq + 1))
                                .font(k.ty.mono)
                                .color(k.c.text_faint),
                        )
                        .child(
                            text(line.text.clone())
                                .font(k.ty.mono)
                                .color(k.c.text_muted)
                                .grow(1.0)
                                .wrap()
                                .selectable(true)
                                .selection_color(k.selection()),
                        )
                })),
        )
}

fn footer(s: &State, k: &Kit) -> El {
    if s.mode == Mode::Backend {
        let saved = if s
            .draft_for(s.selected.unwrap_or_default())
            .is_some_and(|draft| draft.dirty())
        {
            "Unsaved changes stay in this session"
        } else {
            "Saved assignment loaded from the backend"
        };
        return row()
            .w_full()
            .gap(8.0)
            .items_center()
            .child(text(saved).font(k.ty.small).color(k.c.text_faint).grow(1.0))
            .child(
                Button::new("Close")
                    .variant(ButtonVariant::GhostMuted)
                    .on_press(|s: &mut State| s.workbench_open = false)
                    .build(k)
                    .key("workbench-close-button"),
            );
    }

    let stream = Button::new(if s.streaming {
        "Stop stream"
    } else {
        "Stream output"
    })
    .variant(ButtonVariant::Contrast)
    .on_press(|s: &mut State| {
        s.streaming = !s.streaming;
        if s.streaming {
            s.next_output_at = 0.0;
        }
    })
    .build(k)
    .key("workbench-stream");
    let clear = Button::new("Clear")
        .variant(ButtonVariant::GhostMuted)
        .on_press(|s: &mut State| s.output.clear())
        .build(k)
        .key("workbench-clear");
    let close = Button::new("Close")
        .variant(ButtonVariant::GhostMuted)
        .on_press(|s: &mut State| s.workbench_open = false)
        .build(k)
        .key("workbench-close-button");

    row()
        .w_full()
        .gap(8.0)
        .items_center()
        .child(stream)
        .child(clear)
        .child(
            text("Dragging cards changes presentation only · no backend mutation")
                .font(k.ty.small)
                .color(k.c.text_faint)
                .grow(1.0),
        )
        .child(close)
}
