use std::{path::PathBuf, time::Duration};

use gpui_kit::component::{
    Sizable as _, WindowExt as _,
    input::{Input, InputState},
};
use gpui_kit::{Div, WeakEntity};
use waypoints_backend::{Permission, RunEventOrigin};
use waypoints_core::run::{RunState, SnapshotState};

use gpui_kit::base::Disableable as _;
use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    input::{Textarea, TextareaState},
};
use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, ParentElement as _, ScrollHandle,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
};
use waypoints_backend::{Client, RunStatus};

use super::{NodeKey, Target, Workspace, message, run_backend};

const OUTPUT_LIMIT: usize = 128 * 1024;

pub(super) struct RunPanel {
    executable: Entity<InputState>,
    follow_up: Entity<TextareaState>,
    status: Option<RunStatus>,
    polling: bool,
    busy: bool,
    error: Option<String>,
    output: String,
    output_scroll: ScrollHandle,
    cursor: u64,
    output_notice: Option<&'static str>,
}

impl RunPanel {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let executable =
            cx.new(|cx| InputState::new(window, cx).placeholder("/absolute/path/to/agent-acp"));
        let follow_up = cx.new(|cx| TextareaState::new(window, cx));
        Self {
            executable,
            follow_up,
            status: None,
            polling: false,
            busy: false,
            error: None,
            output: String::new(),
            output_scroll: ScrollHandle::new(),
            cursor: 0,
            output_notice: None,
        }
    }
}

fn active(state: &RunState) -> bool {
    matches!(
        state,
        RunState::Starting
            | RunState::Running
            | RunState::AwaitingPermission
            | RunState::Cancelling
            | RunState::Finalizing
    )
}

pub(super) fn state_label(state: &RunState) -> String {
    match state {
        RunState::Starting => "Starting",
        RunState::Running => "Running",
        RunState::AwaitingPermission => "Permission needed",
        RunState::Cancelling => "Cancelling",
        RunState::Finalizing => "Recording revision",
        RunState::Completed => "Completed",
        RunState::Cancelled => "Cancelled",
        RunState::Failed { detail } => return format!("Failed. {detail}"),
        RunState::Interrupted { detail, .. } => return format!("Interrupted. {detail}"),
        RunState::Recovered { .. } => "Recovered",
        _ => "Unknown run state",
    }
    .into()
}

impl Workspace {
    pub(super) fn start_node_run(
        &mut self,
        key: NodeKey,
        continuation: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        let Some(node) = self.node_workspaces.get_mut(&key) else {
            return;
        };
        let panel = &mut node.run;
        if (continuation.is_none()
            && (node.saving || node.draft.read(cx).value().as_ref() != node.saved))
            || panel.busy
            || panel
                .status
                .as_ref()
                .is_some_and(|status| active(&status.run.state))
        {
            return;
        }
        let executable = PathBuf::from(panel.executable.read(cx).value().as_ref().trim());
        let follow_up = panel.follow_up.read(cx).value().to_string();
        if continuation.is_none() && !executable.is_absolute() {
            panel.error = Some("Enter the absolute path to an installed ACP executable.".into());
            cx.notify();
            return;
        }
        panel.busy = true;
        panel.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    run_backend(|client: Client| async move {
                        match continuation {
                            Some(id) => client.continue_run(id, &follow_up).await,
                            None => client.start_run(key.1, key.2, &executable).await,
                        }
                        .map_err(|error| error.to_string())
                    })
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let Some(node) = this.node_workspaces.get_mut(&key) else {
                    return;
                };
                node.run.busy = false;
                match result {
                    Ok(status) => {
                        node.run.status = Some(status);
                        node.run.output.clear();
                        node.run.cursor = 0;
                        node.run.output_notice = None;
                        node.history_stale = true;
                        this.poll_node_run(key, cx);
                    }
                    Err(error) => node.run.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn discover_run(&mut self, key: NodeKey, cx: &mut Context<Self>) {
        let Some(node) = self.node_workspaces.get_mut(&key) else {
            return;
        };
        if node.run.busy || node.run.polling || node.run.status.is_some() {
            return;
        }
        node.run.busy = true;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    run_backend(|client: Client| async move {
                        client
                            .node_runs(key.1, key.2)
                            .await
                            .map(|runs| runs.into_iter().max_by_key(|status| status.run.id))
                            .map_err(|error| error.to_string())
                    })
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let Some(node) = this.node_workspaces.get_mut(&key) else {
                    return;
                };
                node.run.busy = false;
                match result {
                    Ok(status) => {
                        node.run.status = status;
                        this.poll_node_run(key, cx);
                    }
                    Err(error) => node.run.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn poll_node_run(&mut self, key: NodeKey, cx: &mut Context<Self>) {
        let Some(panel) = self.node_workspaces.get_mut(&key).map(|node| &mut node.run) else {
            return;
        };
        let Some(status) = &panel.status else {
            return;
        };
        if panel.polling {
            return;
        }
        panel.polling = true;
        let id = status.run.id;
        let mut cursor = panel.cursor;
        cx.spawn(async move |this, cx| {
            loop {
                let result = cx
                    .background_spawn(async move {
                        run_backend(|client: Client| async move {
                            match client.run_events(id, cursor).await {
                                Ok(page) => Ok((page.status.clone(), Some(page))),
                                Err(_) => client
                                    .run_status(id)
                                    .await
                                    .map(|status| (status, None))
                                    .map_err(|error| error.to_string()),
                            }
                        })
                    })
                    .await;
                let keep_polling = this
                    .update(cx, |this, cx| {
                        let Some(node) = this.node_workspaces.get_mut(&key) else {
                            return false;
                        };
                        let panel = &mut node.run;
                        let keep_polling = match result {
                            Ok((status, page)) => {
                                let keep_polling = active(&status.run.state);
                                if !keep_polling
                                    && panel
                                        .status
                                        .as_ref()
                                        .is_some_and(|old| active(&old.run.state))
                                {
                                    node.history_stale = true;
                                }
                                panel.status = Some(status);
                                if let Some(page) = page {
                                    cursor = page.next;
                                    panel.cursor = cursor;
                                    if page.gap {
                                        panel.output_notice = Some("Some earlier output expired.");
                                    }
                                    for event in page.events {
                                        let update = &event.update;
                                        let kind = update
                                            .get("sessionUpdate")
                                            .and_then(|value| value.as_str())
                                            .unwrap_or("update");
                                        if event.origin == RunEventOrigin::Replay {
                                            panel.output.push_str("[Replayed] ");
                                        }
                                        if let Some(chunk) = update
                                            .pointer("/content/text")
                                            .and_then(|value| value.as_str())
                                        {
                                            panel.output.push_str(chunk);
                                        } else {
                                            panel.output.push('\n');
                                            panel.output.push_str(kind);
                                            if let Some(title) =
                                                update.get("title").and_then(|value| value.as_str())
                                            {
                                                panel.output.push_str(" · ");
                                                panel.output.push_str(title);
                                            }
                                            if let Some(status) = update
                                                .get("status")
                                                .and_then(|value| value.as_str())
                                            {
                                                panel.output.push_str(" · ");
                                                panel.output.push_str(status);
                                            }
                                            panel.output.push('\n');
                                        }
                                    }
                                    if panel.output.len() > OUTPUT_LIMIT {
                                        let boundary = panel
                                            .output
                                            .ceil_char_boundary(panel.output.len() - OUTPUT_LIMIT);
                                        panel.output.drain(..boundary);
                                        panel.output_notice = Some("Some earlier output expired.");
                                    }
                                } else {
                                    panel.output_notice =
                                        Some("The backend no longer has this run’s live output.");
                                }
                                keep_polling
                            }
                            Err(error) => {
                                panel.error = Some(error);
                                false
                            }
                        };
                        panel.polling = keep_polling;
                        cx.notify();
                        keep_polling
                    })
                    .unwrap_or(false);
                if !keep_polling {
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
            }
        })
        .detach();
    }

    fn run_command(
        &mut self,
        key: NodeKey,
        run_id: i64,
        reply: Option<(u64, String)>,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = self.node_workspaces.get_mut(&key).map(|node| &mut node.run) else {
            return;
        };
        if panel.busy {
            return;
        }
        panel.busy = true;
        panel.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    run_backend(|client: Client| async move {
                        match reply {
                            Some((id, option)) => {
                                client.respond_to_permission(run_id, id, &option).await
                            }
                            None => client.cancel_run(run_id).await,
                        }
                        .map_err(|error| error.to_string())
                    })
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let Some(panel) = this.node_workspaces.get_mut(&key).map(|node| &mut node.run)
                else {
                    return;
                };
                panel.busy = false;
                match result {
                    Ok(status) => panel.status = Some(status),
                    Err(error) => panel.error = Some(error),
                }
                this.poll_node_run(key, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn open_permission(
        &self,
        key: NodeKey,
        run_id: i64,
        permission: &Permission,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak = cx.entity().downgrade();
        let request_id = permission.id;
        window.open_dialog(cx, move |dialog, _, cx| {
            let view = weak.upgrade();
            let panel = view
                .as_ref()
                .and_then(|view| view.read(cx).node_workspaces.get(&key))
                .map(|node| &node.run);
            let request = panel
                .and_then(|panel| panel.status.as_ref())
                .filter(|status| status.run.id == run_id)
                .and_then(|status| status.permission.as_ref())
                .filter(|request| request.id == request_id);
            let dialog = dialog.title(format!("Permission for run #{run_id}"));
            let Some(request) = request else {
                return dialog.child(message("This request is no longer pending."));
            };
            let title = request
                .tool_call
                .get("title")
                .and_then(|value| value.as_str())
                .unwrap_or("Agent tool request")
                .to_owned();
            let call = format!("{:#}", request.tool_call);
            let mut content = div().flex().flex_col().gap_2().child(message(title)).child(
                div()
                    .id("permission-tool-call")
                    .max_h_48()
                    .overflow_y_scroll()
                    .child(message(call)),
            );
            if let Some(error) = panel.and_then(|panel| panel.error.clone()) {
                content = content.child(message(error));
            }
            dialog.child(content).footer(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        Button::new("dismiss-permission")
                            .ghost()
                            .label("Dismiss")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .children(request.options.iter().map(|option| {
                        let weak: WeakEntity<Self> = weak.clone();
                        let choice = option.option_id.clone();
                        Button::new(SharedString::from(choice.clone()))
                            .outline()
                            .label(option.name.clone())
                            .disabled(panel.is_some_and(|panel| panel.busy))
                            .on_click(move |_, _, cx| {
                                let _ = weak.update(cx, |this, cx| {
                                    this.run_command(
                                        key,
                                        run_id,
                                        Some((request_id, choice.clone())),
                                        cx,
                                    )
                                });
                            })
                    })),
            )
        });
    }

    pub(in crate::workspace) fn graph_run_status(&self, node: i64) -> Option<&'static str> {
        let (project, workflow) = self.state.selected?;
        let status = self
            .node_workspaces
            .get(&(project, workflow, node))?
            .run
            .status
            .as_ref()?;
        match status.run.state {
            RunState::AwaitingPermission => Some("Permission needed"),
            RunState::Starting | RunState::Running => Some("Running"),
            RunState::Finalizing => Some("Recording revision"),
            RunState::Cancelling => Some("Cancelling"),
            RunState::Completed => Some("Completed"),
            RunState::Failed { .. } => Some("Run failed"),
            RunState::Interrupted { .. } => Some("Interrupted"),
            _ => None,
        }
    }

    pub(in crate::workspace) fn permission_actions(
        &self,
        cx: &Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        let Some((project, workflow)) = self.state.selected else {
            return div();
        };
        let mut actions = div()
            .id("permission-actions")
            .flex()
            .gap_1()
            .overflow_x_scroll()
            .min_w_0();
        for (&key, node) in &self.node_workspaces {
            if (key.0, key.1) != (project, workflow) {
                continue;
            }
            if let Some(status) = &node.run.status
                && let Some(permission) = &status.permission
            {
                let id = status.run.id;
                let permission = permission.clone();
                actions = actions.child(
                    Button::new(SharedString::from(format!("permission-{id}")))
                        .outline()
                        .small()
                        .label(format!("Permission · #{}", key.2))
                        .accessibility_label(format!("Review permission for node #{}", key.2))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_permission(key, id, &permission, window, cx)
                        })),
                );
            }
        }
        div().min_w_0().child(actions)
    }

    pub(super) fn render_run_panel(&self, key: NodeKey, root: Div, cx: &Context<Self>) -> Div {
        let node = &self.node_workspaces[&key];
        let panel = &node.run;
        let status = panel.status.as_ref();
        let running = status.is_some_and(|status| active(&status.run.state));
        let unsaved = node.draft.read(cx).value().as_ref() != node.saved;
        let mut toolbar = div()
            .flex()
            .flex_wrap()
            .gap_2()
            .items_center()
            .child(
                div()
                    .w_56()
                    .child(Input::new(&panel.executable).aria_label("ACP executable")),
            )
            .child(
                Button::new("start-run")
                    .primary()
                    .label("Start run")
                    .disabled(panel.busy || running || unsaved || node.saving)
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.start_node_run(key, None, cx)),
                    ),
            );
        if let Some(status) = status {
            let id = status.run.id;
            if running {
                toolbar = toolbar.child(
                    Button::new("cancel-run")
                        .outline()
                        .label("Cancel run")
                        .disabled(
                            panel.busy
                                || matches!(
                                    status.run.state,
                                    RunState::Finalizing | RunState::Cancelling
                                ),
                        )
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.run_command(key, id, None, cx)),
                        ),
                );
            }
            if !panel.polling {
                toolbar = toolbar.child(
                    Button::new("refresh-run")
                        .ghost()
                        .label("Refresh run")
                        .on_click(cx.listener(move |this, _, _, cx| this.poll_node_run(key, cx))),
                );
            }
        }
        let mut content = div().id("run-output").flex_1().min_h_0().overflow_y_scroll()
            .track_scroll(&panel.output_scroll).flex().flex_col().gap_2()
            .child(message("A fresh run uses the saved assignment and requires a native change, a ready workspace, and an ACP executable."));
        if unsaved {
            content = content.child(message(
                "Save assignment edits before starting a fresh run.",
            ));
        }
        if let Some(error) = &panel.error {
            content = content.child(message(error.clone()));
        }
        if let Some(status) = status {
            let id = status.run.id;
            content = content.child(message(format!(
                "Run #{id} · {}",
                state_label(&status.run.state)
            )));
            match &status.run.snapshot {
                SnapshotState::Saved { .. } => {
                    content = content.child(
                        Button::new("review-run-snapshot")
                            .ghost()
                            .label("Review saved snapshot")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_review(key, Target::Run(id), cx)
                            })),
                    );
                }
                SnapshotState::Failed { detail } => {
                    content = content.child(message(format!("Snapshot failed. {detail}")))
                }
                _ => content = content.child(message("No saved snapshot.")),
            }
            if status.run.session_id.is_some()
                && matches!(
                    status.run.state,
                    RunState::Completed
                        | RunState::Cancelled
                        | RunState::Failed { .. }
                        | RunState::Recovered { .. }
                )
            {
                content = content
                    .child(message("Follow-up instructions"))
                    .child(Textarea::new(&panel.follow_up).aria_label("Follow-up instructions"))
                    .child(
                        Button::new("continue-run")
                            .outline()
                            .label("Continue session")
                            .disabled(panel.busy)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.start_node_run(key, Some(id), cx)
                            })),
                    );
            }
        }
        if let Some(notice) = panel.output_notice {
            content = content.child(message(notice));
        }
        root.child(toolbar)
            .child(content.child(message(panel.output.clone())))
    }
}
