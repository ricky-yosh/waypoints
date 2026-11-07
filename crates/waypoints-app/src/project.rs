//! Screen-space project and workflow navigator for backend-connected runs.

use rune::{Color, Cursor, El, FileDialogOptions, ScrollSpec, Tx, col, el, row, text};
use rune_kit::{Button, ButtonVariant, Kit};

use crate::{LoadState, Mode, State};

pub fn view(s: &State, k: &Kit) -> El {
    if s.mode != Mode::Backend || !s.preferences.sidebar_open {
        return el();
    }
    let (status, status_color) = status(s, k);
    let mut panel = col()
        .key("project-panel")
        .absolute(18.0, 96.0)
        .w(240.0)
        .h(360.0)
        .padding(12.0)
        .gap(8.0)
        .radius(k.radius.lg)
        .fill(k.c.bg_base)
        .border(1.0, k.c.border_base)
        .shadow(2.0, 8.0, Color::BLACK.alpha(0.16))
        .on_press(|_: &mut State| {})
        .on_wheel(|_: &mut State, _, _| true)
        .role("complementary")
        .label("Project navigation")
        .child(
            row()
                .w_full()
                .items_center()
                .justify_between()
                .child(
                    text("Project")
                        .font(k.ty.small_medium)
                        .color(k.c.text_faint),
                )
                .child(
                    row()
                        .padding(3.0)
                        .gap(5.0)
                        .items_center()
                        .radius(999.0)
                        .fill(status_color.alpha(0.14))
                        .child(
                            el().size(5.0, 5.0)
                                .radius(2.5)
                                .fill(status_color)
                                .a11y_hidden(),
                        )
                        .child(
                            text(status)
                                .key("backend-status")
                                .font(k.ty.small)
                                .color(k.c.text_base),
                        ),
                ),
        )
        .child(project_list(s, k));

    if let LoadState::Failed(error) = &s.backend_state {
        panel = panel.child(
            text(error.clone())
                .key("backend-error")
                .font(k.ty.small)
                .color(k.c.danger.fg)
                .w_full()
                .wrap()
                .max_lines(3),
        );
    }
    if matches!(s.backend_state, LoadState::Failed(_) | LoadState::Idle) {
        panel = panel.child(
            Button::new("Retry connection")
                .variant(ButtonVariant::Outline)
                .on_press(|s: &mut State| s.retry_backend())
                .build(k)
                .key("backend-retry"),
        );
    }

    panel = panel.child(
        el().key("project-rule")
            .w_full()
            .h(1.0)
            .fill(k.c.border_base)
            .a11y_hidden(),
    );

    let refresh = Button::new("Refresh")
        .variant(ButtonVariant::GhostMuted)
        .help("Reload the project's workflows")
        .on_press(|s: &mut State| {
            if matches!(s.backend_state, LoadState::Failed(_)) {
                s.retry_backend();
            } else {
                s.load_workflows();
            }
        })
        .build(k)
        .key("workflows-refresh");
    panel = panel.child(
        row()
            .w_full()
            .items_center()
            .justify_between()
            .child(
                text(format!("Workflows · {}", s.workflows.len()))
                    .font(k.ty.small_medium)
                    .color(k.c.text_faint),
            )
            .child(refresh),
    );

    let body = match &s.workflows_state {
        LoadState::Loading => col().key("workflows-loading").padding(8.0).child(
            text("Loading workflows…")
                .font(k.ty.small)
                .color(k.c.text_muted),
        ),
        LoadState::Failed(error) => col()
            .key("workflows-error")
            .padding(8.0)
            .gap(6.0)
            .child(
                text(error.clone())
                    .font(k.ty.small)
                    .color(k.c.danger.fg)
                    .w_full()
                    .wrap()
                    .max_lines(3),
            )
            .child(
                Button::new("Retry workflows")
                    .variant(ButtonVariant::GhostMuted)
                    .on_press(|s: &mut State| s.load_workflows())
                    .build(k)
                    .key("workflows-retry"),
            ),
        _ if s.workflows.is_empty() => col().key("workflows-empty").padding(8.0).child(
            text("No active workflows in this project")
                .font(k.ty.small)
                .color(k.c.text_muted)
                .wrap(),
        ),
        _ => col()
            .key("workflow-list")
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .gap(2.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .children(s.workflows.iter().map(|workflow| {
                let id = workflow.id;
                let selected = s.selected_workflow == Some(id);
                let loading = selected && matches!(s.graph_state, LoadState::Loading);
                el().key(format!("workflow-{id}"))
                    .w_full()
                    .padding(7.0)
                    .radius(k.radius.md)
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
                        "{}{}",
                        workflow.name,
                        if loading { ", loading graph" } else { "" }
                    ))
                    .on_press(move |s: &mut State| s.select_workflow(id))
                    .child(
                        row()
                            .w_full()
                            .items_center()
                            .justify_between()
                            .child(
                                text(workflow.name.clone())
                                    .font(if selected { k.ty.ui_medium } else { k.ty.ui })
                                    .color(if selected {
                                        k.c.text_base
                                    } else {
                                        k.c.text_muted
                                    })
                                    .truncate()
                                    .grow(1.0),
                            )
                            .child(
                                text(if loading { "…" } else { "" })
                                    .font(k.ty.mono)
                                    .color(k.c.text_faint),
                            ),
                    )
            })),
    };
    panel = panel.child(body);

    if let LoadState::Failed(error) = &s.graph_state {
        panel = panel.child(
            col()
                .key("graph-error")
                .gap(5.0)
                .child(
                    text(error.clone())
                        .font(k.ty.small)
                        .color(k.c.danger.fg)
                        .wrap()
                        .max_lines(3),
                )
                .child(
                    Button::new("Retry graph")
                        .variant(ButtonVariant::GhostMuted)
                        .on_press(|s: &mut State| s.load_graph())
                        .build(k)
                        .key("graph-retry"),
                ),
        );
    } else if s.graph_state == LoadState::Ready {
        panel = panel.child(
            text(format!(
                "{} nodes · {} edges",
                s.graph.nodes.len(),
                s.graph.edges.len()
            ))
            .key("graph-count")
            .font(k.ty.small)
            .color(k.c.text_faint),
        );
    }

    panel
}

pub fn open_add_dialog(tx: &mut Tx) {
    let options = FileDialogOptions {
        title: "Add Waypoints project".into(),
        ..FileDialogOptions::default()
    };
    tx.open_folder(options, |s: &mut State, result, _| match result {
        Ok(Some(path)) => s.add_project(path),
        Ok(None) => {}
        Err(error) => s.preferences_error = Some(error.to_string()),
    });
}

fn project_list(s: &State, k: &Kit) -> El {
    let add = Button::new("Add")
        .variant(ButtonVariant::GhostMuted)
        .help("Add an existing project folder")
        .on_press_tx(|_s: &mut State, tx| open_add_dialog(tx))
        .build(k)
        .key("project-add");
    let list_height = (s.preferences.projects.len().max(1) as f32 * 36.0).min(96.0);
    let mut list = col()
        .key("project-list")
        .w_full()
        .h(list_height)
        .gap(2.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    if s.preferences.projects.is_empty() {
        list = list.child(
            text("No projects yet · use Add")
                .key("projects-empty")
                .font(k.ty.small)
                .color(k.c.text_muted),
        );
    } else {
        for (index, project) in s.preferences.projects.iter().enumerate() {
            let selected = s.project_path.as_ref() == Some(project);
            let select_path = project.clone();
            let remove_path = project.clone();
            list = list.child(
                row()
                    .key(format!("project-{index}"))
                    .w_full()
                    .items_center()
                    .gap(4.0)
                    .padding(5.0)
                    .radius(k.radius.sm)
                    .fill(if selected {
                        k.c.layer_02
                    } else {
                        Color::TRANSPARENT
                    })
                    .child(
                        row()
                            .key(format!("project-select-{index}"))
                            .grow(1.0)
                            .min_w(0.0)
                            .items_center()
                            .padding(1.0)
                            .hover(k.c.hover)
                            .focusable(true)
                            .role("button")
                            .label(project.display().to_string())
                            .help(project.display().to_string())
                            .on_press(move |s: &mut State| s.select_project(select_path.clone()))
                            .child(
                                text(project.file_name().map_or_else(
                                    || project.display().to_string(),
                                    |name| name.to_string_lossy().into_owned(),
                                ))
                                .font(if selected { k.ty.ui_medium } else { k.ty.ui })
                                .color(if selected {
                                    k.c.text_base
                                } else {
                                    k.c.text_muted
                                })
                                .truncate()
                                .grow(1.0),
                            ),
                    )
                    .child(
                        Button::new("×")
                            .variant(ButtonVariant::GhostFaint)
                            .help("Remove this project from the list")
                            .on_press(move |s: &mut State| s.remove_project(&remove_path))
                            .build(k)
                            .key(format!("project-remove-{index}")),
                    ),
            );
        }
    }
    col()
        .w_full()
        .gap(4.0)
        .child(
            row()
                .w_full()
                .items_center()
                .justify_between()
                .child(
                    text(format!("Projects · {}", s.preferences.projects.len()))
                        .font(k.ty.small_medium)
                        .color(k.c.text_faint),
                )
                .child(add),
        )
        .child(list)
}

fn status(s: &State, k: &Kit) -> (&'static str, Color) {
    match &s.backend_state {
        LoadState::Ready => ("Connected", k.c.success.fg),
        LoadState::Loading => ("Connecting", k.c.warning.fg),
        LoadState::Failed(_) => ("Unavailable", k.c.danger.fg),
        LoadState::Idle => ("Idle", k.c.text_faint),
    }
}
