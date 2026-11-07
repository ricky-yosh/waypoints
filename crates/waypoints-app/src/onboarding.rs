//! Functional onboarding for the canvas: project drop, agent-led creation copy,
//! and a persistent New workflow affordance.
//!
//! This module contains no decorative background animation. The empty states'
//! first job is to be keyboard accessible and honest about loading, failure, and
//! empty results.

use rune::{Align, Color, El, TextStyle, col, el, row, text};
use rune_kit::{Button, ButtonVariant, IconButton, Kit, glyphs};

use crate::{LoadState, Mode, State};

/// The exact prompt shown in the empty-workflow state and the persistent help.
pub(crate) const WORKFLOW_PROMPT: &str = "Use waypoints to make a workflow and create this feature";

/// The functional state, if the canvas should be replaced by onboarding.
pub fn view(s: &State, k: &Kit, size: [f32; 2], insets: [f32; 2], dark: bool) -> Option<El> {
    if s.mode != Mode::Backend || s.display_fixture {
        return None;
    }
    if s.project_path.is_none() {
        return Some(no_project(s, k, size, insets, dark));
    }
    // Once a graph has been shown, transient disconnects keep the canvas and
    // drafts visible; the sidebar reports "Reconnecting" and the sync error.
    let has_canvas = s.source_graph.is_some() || !s.graph.nodes.is_empty();
    match &s.backend_state {
        LoadState::Failed(error) if !has_canvas => Some(failure(
            s,
            k,
            size,
            insets,
            dark,
            "onboarding-error",
            "Can’t reach the Waypoints backend.",
            error,
            true,
        )),
        LoadState::Failed(_) => None,
        LoadState::Loading | LoadState::Idle if !has_canvas => Some(status(
            s,
            k,
            size,
            insets,
            dark,
            "onboarding-loading",
            "Connecting to the backend…",
            "Existing workflows are never changed by opening the app.",
        )),
        LoadState::Loading | LoadState::Idle => None,
        LoadState::Ready => match &s.workflows_state {
            LoadState::Failed(error) if !has_canvas => Some(failure(
                s,
                k,
                size,
                insets,
                dark,
                "onboarding-workflows-error",
                "Couldn’t load this project’s workflows.",
                error,
                false,
            )),
            LoadState::Failed(_) => None,
            LoadState::Loading if !has_canvas => Some(status(
                s,
                k,
                size,
                insets,
                dark,
                "onboarding-loading",
                "Loading workflows…",
                "The last known canvas is replaced only when a project is selected.",
            )),
            LoadState::Loading => None,
            LoadState::Idle if !has_canvas => Some(status(
                s,
                k,
                size,
                insets,
                dark,
                "onboarding-loading",
                "Loading workflows…",
                "The backend connection is not ready yet.",
            )),
            LoadState::Idle => None,
            LoadState::Ready if s.workflows.is_empty() && !has_canvas => {
                Some(no_workflows(s, k, size, insets, dark))
            }
            LoadState::Ready if !has_canvas => match &s.graph_state {
                LoadState::Idle | LoadState::Loading => Some(status(
                    s,
                    k,
                    size,
                    insets,
                    dark,
                    "onboarding-graph-loading",
                    "Loading workflow…",
                    "The graph is fetched separately from the project list.",
                )),
                LoadState::Failed(error) => Some(graph_failure(s, k, size, insets, dark, error)),
                LoadState::Ready => None,
            },
            LoadState::Ready => None,
        },
    }
}

/// Persistent sheet used by the sidebar and HUD New workflow controls.
pub fn workflow_help(s: &State, k: &Kit, size: [f32; 2]) -> El {
    let width = (size[0] - 48.0).clamp(360.0, 540.0);
    let panel = col()
        .key("workflow-help")
        .w(width)
        .padding(20.0)
        .gap(12.0)
        .radius(k.radius.xl)
        .fill(k.c.bg_base)
        .border(1.0, k.c.border_base)
        .shadow(8.0, 32.0, Color::BLACK.alpha(0.28))
        .on_press(|_: &mut State| {})
        .on_wheel(|_: &mut State, _, _| true)
        .role("dialog")
        .label("New workflow instructions")
        .focus_return(
            s.workflow_help_return
                .clone()
                .unwrap_or_else(|| "new-workflow-hud".into()),
        )
        .child(
            row()
                .w_full()
                .items_center()
                .justify_between()
                .child(
                    text("New workflow")
                        .font(TextStyle::new(18.0, 620.0))
                        .color(k.c.text_base),
                )
                .child(
                    IconButton::new(&glyphs::CLOSE)
                        .variant(rune_kit::IconButtonVariant::GhostMuted)
                        .label("Close new workflow instructions")
                        .on_press(|s: &mut State| s.close_workflow_help())
                        .build(k)
                        .key("workflow-help-close"),
                ),
        )
        .child(instructions(s, k));
    overlay(panel, k, |s: &mut State| s.close_workflow_help())
}

fn no_project(s: &State, k: &Kit, size: [f32; 2], insets: [f32; 2], dark: bool) -> El {
    let width = (size[0] - 80.0).clamp(360.0, 540.0);
    el()
        .key("onboarding-no-project")
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .px(insets[0])
        .py(insets[1])
        .center()
        .fill(theme_bg(k, dark))
        .child(
            col()
                .w(width)
                .gap(14.0)
                .items_center()
                .child(
                    text("Open a project to see its workflows.")
                        .key("onboarding-title")
                        .font(TextStyle::new(26.0, 620.0))
                        .color(k.c.text_base)
                        .align(Align::Center),
                )
                .child(
                    text("Workflows are created by your agent harness. This window follows the saved work.")
                        .font(k.ty.ui)
                        .color(k.c.text_muted)
                        .align(Align::Center)
                        .wrap(),
                )
                .child(
                    el()
                        .key("project-drop-target")
                        .w_full()
                        .h(156.0)
                        .center()
                        .radius(k.radius.xl)
                        .fill(if s.drop_hover {
                            k.c.bg_accent.alpha(0.18)
                        } else {
                            k.c.layer_01
                        })
                        .border(
                            if s.drop_hover { 2.0 } else { 1.0 },
                            if s.drop_hover {
                                k.c.bg_accent
                            } else {
                                k.c.border_muted
                            },
                        )
                        .on_drop_files(|s: &mut State, files, _| s.drop_project(files))
                        .on_drop_hover(|s: &mut State, over, _| s.drop_hover = over)
                        .role("group")
                        .label("Project folder drop target")
                        .child(
                            col()
                                .gap(6.0)
                                .items_center()
                                .child(
                                    text("Drop your project folder here")
                                        .font(k.ty.ui_medium)
                                        .color(k.c.text_base),
                                )
                                .child(
                                    text("or choose a folder below")
                                        .font(k.ty.small)
                                        .color(k.c.text_faint),
                                ),
                        ),
                )
                .child(
                    Button::new("Open project…")
                        .variant(ButtonVariant::Contrast)
                        .on_press_tx(|s: &mut State, tx| {
                            s.project_open_error = None;
                            crate::project::open_add_dialog(tx);
                        })
                        .build(k)
                        .key("onboarding-open-project"),
                )
                .children(s.project_open_error.as_ref().map(|error| {
                    text(error.clone())
                        .key("project-drop-error")
                        .font(k.ty.small)
                        .color(k.c.danger.fg)
                        .align(Align::Center)
                        .wrap()
                        .max_lines(3)
                }))
                .child(
                    text("Opening a folder only adds it to this app. It never creates a workflow or initializes Git/jj.")
                        .font(k.ty.small)
                        .color(k.c.text_faint)
                        .align(Align::Center)
                        .wrap(),
                ),
        )
}

fn no_workflows(s: &State, k: &Kit, size: [f32; 2], insets: [f32; 2], dark: bool) -> El {
    let project = s
        .project_path
        .as_ref()
        .map_or_else(String::new, |path| path.display().to_string());
    let width = (size[0] - 80.0).clamp(360.0, 560.0);
    el().key("onboarding-no-workflows")
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .px(insets[0])
        .py(insets[1])
        .center()
        .fill(theme_bg(k, dark))
        .child(
            col()
                .w(width)
                .gap(12.0)
                .child(
                    text("To create a workflow, ask your AI agent to do so using Waypoints.")
                        .key("onboarding-title")
                        .font(TextStyle::new(24.0, 620.0))
                        .color(k.c.text_base)
                        .wrap(),
                )
                .child(
                    text(project.clone())
                        .font(k.ty.mono)
                        .color(k.c.text_faint)
                        .help(project)
                        .truncate(),
                )
                .child(instructions(s, k))
                .child(
                    row()
                        .w_full()
                        .gap(8.0)
                        .items_center()
                        .child(
                            Button::new("Choose another project…")
                                .variant(ButtonVariant::GhostMuted)
                                .on_press_tx(|s: &mut State, tx| {
                                    s.project_open_error = None;
                                    crate::project::open_add_dialog(tx);
                                })
                                .build(k)
                                .key("onboarding-choose-project"),
                        )
                        .children(s.sync_error.as_ref().map(|error| {
                            text(error.clone())
                                .key("onboarding-sync-error")
                                .font(k.ty.small)
                                .color(k.c.warning.fg)
                                .wrap()
                                .max_lines(2)
                        })),
                )
                .children(s.project_open_error.as_ref().map(|error| {
                    text(error.clone())
                        .key("project-drop-error")
                        .font(k.ty.small)
                        .color(k.c.danger.fg)
                        .wrap()
                        .max_lines(3)
                })),
        )
}

fn instructions(s: &State, k: &Kit) -> El {
    col()
        .key("workflow-instructions")
        .w_full()
        .gap(8.0)
        .child(
            text("In your agent harness, open this repository and paste this prompt.")
                .font(k.ty.ui)
                .color(k.c.text_muted)
                .wrap(),
        )
        .child(
            row()
                .w_full()
                .gap(8.0)
                .items_center()
                .child(
                    el().grow(1.0)
                        .min_w(0.0)
                        .padding(10.0)
                        .radius(k.radius.md)
                        .fill(k.c.layer_01)
                        .border(1.0, k.c.border_base)
                        .child(
                            text(WORKFLOW_PROMPT)
                                .key("workflow-prompt")
                                .font(k.ty.mono)
                                .color(k.c.text_base)
                                .selectable(true)
                                .wrap(),
                        ),
                )
                .child(
                    Button::new("Copy prompt")
                        .variant(ButtonVariant::Outline)
                        .help("Copy the exact prompt to the clipboard")
                        .on_press_tx(|s: &mut State, tx| {
                            tx.copy(WORKFLOW_PROMPT);
                            s.prompt_notice = Some("Prompt copied to clipboard".into());
                        })
                        .build(k)
                        .key("copy-workflow-prompt"),
                ),
        )
        .child(
            text("Add the feature description before sending. Copying never starts work.")
                .font(k.ty.small)
                .color(k.c.text_faint)
                .wrap(),
        )
        .children(s.prompt_notice.as_ref().map(|notice| {
            text(notice.clone())
                .key("workflow-prompt-copy-status")
                .font(k.ty.small)
                .color(k.c.success.fg)
                .role("status")
        }))
}

#[allow(clippy::too_many_arguments)]
fn graph_failure(
    s: &State,
    k: &Kit,
    size: [f32; 2],
    insets: [f32; 2],
    dark: bool,
    error: &str,
) -> El {
    message(
        k,
        insets,
        size,
        dark,
        "onboarding-graph-error",
        "Couldn’t load this workflow’s graph.",
        error,
        Some(
            row()
                .gap(8.0)
                .child(
                    Button::new("Retry graph")
                        .variant(ButtonVariant::Outline)
                        .on_press(|s: &mut State| s.load_graph())
                        .build(k)
                        .key("onboarding-graph-retry"),
                )
                .child(
                    Button::new("Choose another project…")
                        .variant(ButtonVariant::GhostMuted)
                        .on_press_tx(|s: &mut State, tx| {
                            s.project_open_error = None;
                            crate::project::open_add_dialog(tx);
                        })
                        .build(k)
                        .key("onboarding-choose-project"),
                )
                .children(s.project_open_error.as_ref().map(|error| {
                    text(error.clone())
                        .key("project-drop-error")
                        .font(k.ty.small)
                        .color(k.c.danger.fg)
                        .wrap()
                        .max_lines(2)
                })),
        ),
    )
}

#[allow(clippy::too_many_arguments)]
fn status(
    _s: &State,
    k: &Kit,
    size: [f32; 2],
    insets: [f32; 2],
    dark: bool,
    key: &'static str,
    title: &str,
    detail: &str,
) -> El {
    message(k, insets, size, dark, key, title, detail, None)
}

#[allow(clippy::too_many_arguments)]
fn failure(
    s: &State,
    k: &Kit,
    size: [f32; 2],
    insets: [f32; 2],
    dark: bool,
    key: &'static str,
    title: &str,
    detail: &str,
    backend: bool,
) -> El {
    let retry = if backend {
        Button::new("Retry connection")
            .variant(ButtonVariant::Outline)
            .on_press(|s: &mut State| s.retry_backend())
            .build(k)
            .key("onboarding-retry")
    } else {
        Button::new("Retry workflows")
            .variant(ButtonVariant::Outline)
            .on_press(|s: &mut State| s.load_workflows())
            .build(k)
            .key("onboarding-retry")
    };
    message(
        k,
        insets,
        size,
        dark,
        key,
        title,
        detail,
        Some(
            row()
                .gap(8.0)
                .child(retry)
                .child(
                    Button::new("Choose another project…")
                        .variant(ButtonVariant::GhostMuted)
                        .on_press_tx(|s: &mut State, tx| {
                            s.project_open_error = None;
                            crate::project::open_add_dialog(tx);
                        })
                        .build(k)
                        .key("onboarding-choose-project"),
                )
                .children(s.project_open_error.as_ref().map(|error| {
                    text(error.clone())
                        .key("project-drop-error")
                        .font(k.ty.small)
                        .color(k.c.danger.fg)
                        .wrap()
                        .max_lines(2)
                })),
        ),
    )
}

#[allow(clippy::too_many_arguments)]
fn message(
    k: &Kit,
    insets: [f32; 2],
    size: [f32; 2],
    dark: bool,
    key: &'static str,
    title: &str,
    detail: &str,
    actions: Option<El>,
) -> El {
    let width = (size[0] - 80.0).clamp(320.0, 500.0);
    el().key(key)
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .px(insets[0])
        .py(insets[1])
        .center()
        .fill(theme_bg(k, dark))
        .child(
            col()
                .w(width)
                .gap(10.0)
                .items_center()
                .child(
                    text(title)
                        .font(TextStyle::new(22.0, 620.0))
                        .color(k.c.text_base)
                        .align(Align::Center)
                        .wrap(),
                )
                .child(
                    text(detail)
                        .font(k.ty.ui)
                        .color(k.c.text_muted)
                        .help(detail.to_owned())
                        .align(Align::Center)
                        .wrap()
                        .max_lines(4),
                )
                .children(actions),
        )
}

fn overlay(panel: El, k: &Kit, close: fn(&mut State)) -> El {
    el().key("workflow-help-overlay")
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .center()
        .fill(k.c.scrim)
        .modal(true, close)
        .child(panel)
}

fn theme_bg(k: &Kit, dark: bool) -> Color {
    // The Kit palette already follows the resolved appearance; this helper keeps
    // the call sites explicit about the currently resolved theme.
    let _ = dark;
    k.c.bg_base
}
