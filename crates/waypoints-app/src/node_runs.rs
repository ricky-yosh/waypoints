use std::{cell::Cell, collections::VecDeque, path::PathBuf, rc::Rc, time::Duration};

use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable as _;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Selectable as _, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    input::{Input, InputState, Textarea, TextareaState},
    message_scroller::{MessageScroller, MessageScrollerState},
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Context, Div, Entity, InteractiveElement as _,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, WeakEntity,
    Window, div,
};
use waypoints_backend::{Client, Permission, RunEvent, RunEventOrigin, RunStatus};
use waypoints_core::run::{RunState, SnapshotState};

use super::{
    NodeKey, Target, Workspace, current_review_changed, markdown_with_copy, message, run_backend,
    run_state_visual,
};

const OUTPUT_LIMIT: usize = 128 * 1024;

const OUTPUT_BLOCK_LIMIT: usize = 512;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stream {
    Assistant,
    Thought,
}

enum Activity {
    Text(Stream, String),
    Tool {
        id: String,
        fields: serde_json::Value,
    },
    Other(serde_json::Value),
}

struct ActivityBlock {
    sequence: u64,
    origin: RunEventOrigin,
    activity: Activity,
    expanded: bool,
}

#[derive(Default)]
struct Transcript {
    blocks: VecDeque<ActivityBlock>,
    break_stream: bool,
    trimmed: bool,
}

impl Transcript {
    fn push(&mut self, sequence: u64, origin: RunEventOrigin, update: serde_json::Value) -> u64 {
        let mut changed = sequence;
        let kind = update
            .get("sessionUpdate")
            .and_then(serde_json::Value::as_str);
        let stream = match kind {
            Some("agent_message_chunk") => Some(Stream::Assistant),
            Some("agent_thought_chunk") => Some(Stream::Thought),
            _ => None,
        };
        if let Some(stream) = stream
            && let Some(text) = update
                .pointer("/content/text")
                .and_then(serde_json::Value::as_str)
        {
            if !self.break_stream
                && let Some(previous) = self.blocks.back_mut()
                && previous.origin == origin
                && let Activity::Text(previous_stream, previous_text) = &mut previous.activity
                && *previous_stream == stream
            {
                previous_text.push_str(text);
                changed = previous.sequence;
            } else {
                self.add(sequence, origin, Activity::Text(stream, text.into()));
            }
            self.break_stream = false;
        } else if matches!(kind, Some("tool_call" | "tool_call_update"))
            && let Some(id) = update.get("toolCallId").and_then(serde_json::Value::as_str)
        {
            self.break_stream = true;
            if let Some((existing_sequence, fields)) =
                self.blocks.iter_mut().rev().find_map(|block| {
                    if block.origin == origin
                        && let Activity::Tool {
                            id: existing,
                            fields,
                        } = &mut block.activity
                        && existing == id
                    {
                        Some((block.sequence, fields))
                    } else {
                        None
                    }
                })
            {
                changed = existing_sequence;
                if let (Some(fields), Some(update)) = (fields.as_object_mut(), update.as_object()) {
                    fields.extend(
                        update
                            .iter()
                            .map(|(key, value)| (key.clone(), value.clone())),
                    );
                }
            } else {
                self.add(
                    sequence,
                    origin,
                    Activity::Tool {
                        id: id.to_owned(),
                        fields: update,
                    },
                );
            }
        } else {
            self.break_stream = true;
            self.add(sequence, origin, Activity::Other(update));
        }
        self.trim();
        changed
    }

    fn apply_events(
        &mut self,
        events: Vec<RunEvent>,
        gap: bool,
        scroll: &Entity<MessageScrollerState>,
        cx: &mut App,
    ) {
        self.break_stream |= gap;
        if events.is_empty() {
            return;
        }
        let previous: Vec<_> = self.blocks.iter().map(|block| block.sequence).collect();
        let mut changed = Vec::new();
        for event in events {
            let sequence = self.push(event.sequence, event.origin, event.update);
            if let Ok(index) = previous.binary_search(&sequence) {
                changed.push(index);
            }
        }
        let removed = self.blocks.front().map_or(previous.len(), |first| {
            previous.partition_point(|sequence| *sequence < first.sequence)
        });
        let appended = self.blocks.len() - (previous.len() - removed);
        changed.sort_unstable();
        changed.dedup();
        scroll.update(cx, |scroll, cx| {
            if removed > 0 {
                scroll.splice(0..removed, 0, cx);
            }
            if appended > 0 {
                scroll.append(appended, cx);
            }
            for index in changed
                .into_iter()
                .filter_map(|index| index.checked_sub(removed))
            {
                scroll.remeasure_items(index..index + 1, cx);
            }
        });
    }

    fn add(&mut self, sequence: u64, origin: RunEventOrigin, activity: Activity) {
        self.blocks.push_back(ActivityBlock {
            sequence,
            origin,
            activity,
            expanded: false,
        });
    }

    fn trim(&mut self) {
        let mut remaining = OUTPUT_LIMIT;
        let mut keep = 0;
        for block in self.blocks.iter_mut().rev().take(OUTPUT_BLOCK_LIMIT) {
            let size = match &mut block.activity {
                Activity::Text(_, text) => {
                    if keep == 0 && text.len() > remaining {
                        let boundary = text.floor_char_boundary(remaining);
                        text.truncate(boundary);
                        self.trimmed = true;
                        self.break_stream = true;
                    }
                    text.len()
                }
                Activity::Tool { fields, .. } | Activity::Other(fields) => fields.to_string().len(),
            };
            if size > remaining {
                break;
            }
            remaining -= size;
            keep += 1;
        }
        let drop = self.blocks.len() - keep;
        if drop > 0 {
            self.blocks.drain(..drop);
            self.trimmed = true;
        }
    }
}

struct PermissionReply {
    request_id: u64,
    choice: String,
    window: AnyWindowHandle,
    dialog_open: Rc<Cell<bool>>,
}

pub(super) struct RunPanel {
    executable: Entity<InputState>,
    follow_up: Entity<TextareaState>,
    status: Option<RunStatus>,
    polling: bool,
    checked: bool,
    busy: bool,
    error: Option<String>,
    output: Transcript,
    output_scroll: Entity<MessageScrollerState>,
    cursor: u64,
    output_notice: Option<&'static str>,
}

impl RunPanel {
    pub(super) fn continue_target(&self) -> Option<i64> {
        self.status
            .as_ref()
            .filter(|status| can_continue(status))
            .map(|status| status.run.id)
    }

    pub(super) fn is_busy(&self) -> bool {
        self.busy
    }

    #[cfg(feature = "screenshot-fixtures")]
    pub(super) fn install_qa_permission(&mut self, node_id: i64, run_id: i64, title: &str) {
        self.status = Some(
            serde_json::from_value(serde_json::json!({
                "run": {
                    "id": run_id,
                    "workflow_id": 7,
                    "node_id": node_id,
                    "prompt": title,
                    "directory": "/tmp/waypoints-native-qa/workspace",
                    "change_id": "qa-change",
                    "executable": "/tmp/waypoints-native-qa/fake-acp",
                    "state": {"status": "awaiting_permission"},
                    "snapshot": {"status": "not_recorded"}
                },
                "permission": {
                    "id": run_id,
                    "tool_call": {"title": title},
                    "options": [
                        {"option_id": "deny", "name": "Deny once", "kind": "reject_once"}
                    ]
                }
            }))
            .expect("native QA permission fixture matches the backend schema"),
        );
        self.busy = false;
        self.polling = false;
        self.error = None;
    }

    #[cfg(feature = "native-qa")]
    pub(super) fn set_qa_idle(&mut self) {
        self.busy = false;
        self.polling = false;
        self.error = None;
    }

    #[cfg(feature = "native-qa")]
    pub(super) fn clear_qa_permission(&mut self) {
        if let Some(status) = &mut self.status {
            status.permission = None;
        }
    }

    pub(super) fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let executable =
            cx.new(|cx| InputState::new(window, cx).placeholder("/absolute/path/to/agent-acp"));
        let follow_up =
            cx.new(|cx| TextareaState::new(window, cx).placeholder("Follow-up instructions"));
        Self {
            executable,
            follow_up,
            status: None,
            polling: false,
            checked: false,
            busy: false,
            error: None,
            output: Transcript::default(),
            output_scroll: cx.new(|cx| MessageScrollerState::new(0, cx)),
            cursor: 0,
            output_notice: None,
        }
    }

    #[cfg(feature = "screenshot-fixtures")]
    pub(super) fn set_screenshot_phase(
        &mut self,
        id: i64,
        state: RunState,
        cx: &mut gpui_kit::App,
    ) {
        if let Some(status) = &mut self.status {
            status.run.node_id = id;
            status.run.id += id;
            if state != RunState::Completed {
                status.run.snapshot = SnapshotState::NotRecorded;
            }
            status.run.state = state.clone();
        }
        match state {
            RunState::Running => {
                if let Some(tool) = self
                    .output
                    .blocks
                    .iter_mut()
                    .find(|block| block.sequence == 6)
                {
                    tool.expanded = true;
                }
            }
            RunState::Completed => {
                self.output = Transcript::default();
                self.output.push(
                    1,
                    RunEventOrigin::Live,
                    serde_json::json!({
                        "sessionUpdate": "tool_call",
                        "toolCallId": "fixture-test",
                        "title": "Verify stale review handling",
                        "status": "failed"
                    }),
                );
            }
            _ => self.output = Transcript::default(),
        }
        self.output_scroll = cx.new(|cx| MessageScrollerState::new(self.output.blocks.len(), cx));
    }

    #[cfg(feature = "screenshot-fixtures")]
    pub(super) fn set_screenshot_unrun(&mut self) {
        self.checked = true;
    }

    #[cfg(feature = "screenshot-fixtures")]
    pub(super) fn install_screenshot_fixture(&mut self, cx: &mut gpui_kit::App) {
        use serde_json::json;

        self.status = Some(
            serde_json::from_value(json!({
                "run": {
                    "id": 42,
                    "workflow_id": 7,
                    "node_id": 1,
                    "prompt": "Polish the node workspace.",
                    "directory": "/tmp/waypoints-screenshots/workspace",
                    "change_id": "screenshot-change",
                    "executable": "/usr/local/bin/codex-acp",
                    "session_id": "screenshot-session",
                    "continued_from": null,
                    "process_id": null,
                    "state": {"status": "completed"},
                    "snapshot": {
                        "status": "saved",
                        "revision": {
                            "change_id": "pqrstuvwxyzabcdefghijklmnop",
                            "commit_id": "0123456789abcdef0123456789abcdef01234567",
                            "description": "Polish the node workspace",
                            "conflicted": false
                        }
                    }
                },
                "permission": null
            }))
            .expect("screenshot run status matches the backend schema"),
        );
        self.polling = false;
        self.output_notice =
            Some("Earlier live updates expired; this transcript starts after the gap.");
        let mut transcript = Transcript::default();
        transcript.push(
            1,
            RunEventOrigin::Replay,
            json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "Earlier in this session I inspected the review target helpers and kept the existing diff component.\n\n"}}),
        );
        transcript.push(
            2,
            RunEventOrigin::Live,
            json!({"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "Keep the retained DiffState; adjust presentation and freshness only."}}),
        );
        transcript.push(
            3,
            RunEventOrigin::Live,
            json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "## Review workspace\n\nThe comparison target is explicit, and each file keeps its own `DiffState`.\n\n```rust\nfn target_label(target: &Target) -> String {\n    match target {\n        Target::Current => \"Current recorded revision\".into(),\n        Target::Run(id) => format!(\"Run #{id} snapshot\"),\n        Target::Revision(commit) => format!(\"Native revision {}\", short_id(commit)),\n    }\n}\n```\n\nNext I checked the output transcript and history metadata.\n\n"}}),
        );
        transcript.push(
            4,
            RunEventOrigin::Live,
            json!({
                "sessionUpdate": "tool_call",
                "toolCallId": "tool-read",
                "title": "Read changed files",
                "kind": "read",
                "status": "pending",
                "locations": [
                    {"path": "crates/waypoints-app/src/node_workspace.rs"},
                    {"path": "crates/waypoints-app/src/node_runs.rs"}
                ]
            }),
        );
        transcript.push(
            5,
            RunEventOrigin::Live,
            json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": "tool-read",
                "status": "completed",
                "content": [{"type": "content", "content": {"type": "text", "text": "Read 2 files"}}]
            }),
        );
        transcript.push(
            6,
            RunEventOrigin::Live,
            json!({
                "sessionUpdate": "tool_call",
                "toolCallId": "tool-check",
                "title": "Run focused tests",
                "kind": "execute",
                "status": "completed",
                "rawInput": {"command": "cargo test -p waypoints-app --locked node_workspace"}
            }),
        );
        transcript.push(
            7,
            RunEventOrigin::Live,
            json!({"sessionUpdate": "usage_update", "used": 48210, "size": 200000}),
        );
        transcript.push(
            8,
            RunEventOrigin::Live,
            json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "The focused tests pass. The diff viewport still owns its scrolling, and historical targets remain pinned.\n"}}),
        );
        self.output = transcript;
        self.output_scroll = cx.new(|cx| MessageScrollerState::new(self.output.blocks.len(), cx));
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

pub(super) fn can_continue(status: &RunStatus) -> bool {
    status.run.session_id.is_some()
        && matches!(
            status.run.state,
            RunState::Completed
                | RunState::Cancelled
                | RunState::Failed { .. }
                | RunState::Recovered { .. }
        )
}

pub(super) fn compact_follow_up(window: &Window) -> bool {
    window.viewport_size().height < window.rem_size() * 35.0
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
                        node.run.output = Transcript::default();
                        node.run
                            .output_scroll
                            .update(cx, |scroll, cx| scroll.reset(0, cx));
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
                        node.run.checked = true;
                        let discovered = status.clone();
                        node.run.status = status;
                        if let Some(status) = &discovered {
                            node.mark_review_stale_if_snapshot_changed(status);
                        }
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
                                    if let SnapshotState::Saved { revision } = &status.run.snapshot
                                    {
                                        node.current_review_stale = current_review_changed(
                                            &node.target,
                                            &node.review,
                                            &revision.commit_id,
                                        );
                                    }
                                }
                                panel.status = Some(status);
                                if let Some(page) = page {
                                    cursor = page.next;
                                    panel.cursor = cursor;
                                    if page.gap {
                                        panel.output_notice = Some("Some earlier output expired.");
                                    }
                                    panel.output.apply_events(
                                        page.events,
                                        page.gap,
                                        &panel.output_scroll,
                                        cx,
                                    );
                                    if panel.output.trimmed {
                                        panel.output_notice =
                                            Some("Some earlier output was omitted.");
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
        reply: Option<PermissionReply>,
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
            let (answer, dialog) = match reply {
                Some(reply) => (
                    Some((reply.request_id, reply.choice)),
                    Some((reply.window, reply.dialog_open)),
                ),
                None => (None, None),
            };
            let result = cx
                .background_spawn(async move {
                    run_backend(|client: Client| async move {
                        match answer {
                            Some((id, option)) => {
                                client.respond_to_permission(run_id, id, &option).await
                            }
                            None => client.cancel_run(run_id).await,
                        }
                        .map_err(|error| error.to_string())
                    })
                })
                .await;
            let answered = result.is_ok();
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
            if answered
                && let Some((window, dialog_open)) = dialog
                && dialog_open.replace(false)
            {
                let _ = window.update(cx, |_, window, cx| window.close_dialog(cx));
            }
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
        let dialog_open = Rc::new(Cell::new(true));
        window.open_dialog(cx, move |dialog, _, cx| {
            let close_flag = dialog_open.clone();
            let dialog = dialog.on_close(move |_, _, _| close_flag.set(false));
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
                            .on_click({
                                let dialog_open = dialog_open.clone();
                                move |_, window, cx| {
                                    dialog_open.set(false);
                                    window.close_dialog(cx);
                                }
                            }),
                    )
                    .children(request.options.iter().map(|option| {
                        let weak: WeakEntity<Self> = weak.clone();
                        let choice = option.option_id.clone();
                        let dialog_open = dialog_open.clone();
                        Button::new(SharedString::from(choice.clone()))
                            .outline()
                            .label(option.name.clone())
                            .disabled(panel.is_some_and(|panel| panel.busy))
                            .on_click(move |_, window, cx| {
                                let reply = PermissionReply {
                                    request_id,
                                    choice: choice.clone(),
                                    window: window.window_handle(),
                                    dialog_open: dialog_open.clone(),
                                };
                                let _ = weak.update(cx, |this, cx| {
                                    this.run_command(key, run_id, Some(reply), cx);
                                });
                            })
                    })),
            )
        });
    }

    pub(super) fn open_follow_up(
        &self,
        key: NodeKey,
        run_id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let dialog = dialog.title(format!("Continue run #{run_id}"));
            let view = weak.upgrade();
            let panel = view
                .as_ref()
                .and_then(|view| view.read(cx).node_workspaces.get(&key))
                .map(|node| &node.run);
            let Some(panel) = panel.filter(|panel| {
                panel.status.as_ref().is_some_and(|status| {
                    status.run.id == run_id && status.run.session_id.is_some()
                })
            }) else {
                return dialog.child(message("This run is no longer available for continuation."));
            };
            let submit_weak = weak.clone();
            dialog
                .child(message("Send follow-up instructions to the saved session."))
                .child(
                    Textarea::new(&panel.follow_up)
                        .h(gpui_kit::rems(6.))
                        .aria_label("Follow-up instructions"),
                )
                .footer(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            Button::new("cancel-follow-up")
                                .ghost()
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("submit-follow-up")
                                .primary()
                                .icon(IconName::Play)
                                .label("Continue session")
                                .disabled(panel.busy)
                                .on_click(move |_, window, cx| {
                                    let _ = submit_weak.update(cx, |this, cx| {
                                        this.continue_node_run(key, run_id, cx);
                                    });
                                    window.close_dialog(cx);
                                }),
                        ),
                )
        });
    }

    pub(in crate::workspace) fn graph_activity(
        &self,
        key: NodeKey,
    ) -> super::super::workflow_graph::GraphActivity<'_> {
        use super::super::workflow_graph::GraphActivity;

        let Some(panel) = self.node_workspaces.get(&key).map(|node| &node.run) else {
            return GraphActivity::default();
        };
        let blocks = &panel.output.blocks;
        let assistant = blocks.iter().rev().find_map(|block| match &block.activity {
            Activity::Text(Stream::Assistant, text) if !text.trim().is_empty() => {
                Some(text.as_str())
            }
            _ => None,
        });
        let tool = blocks.iter().rev().find_map(|block| match &block.activity {
            Activity::Tool { fields, .. } => Some(tool_heading(fields, "Tool call")),
            _ => None,
        });
        let failed_tool = blocks.iter().rev().find_map(|block| match &block.activity {
            Activity::Tool { fields, .. }
                if fields.get("status").and_then(serde_json::Value::as_str) == Some("failed") =>
            {
                Some(tool_heading(fields, "Tool call"))
            }
            _ => None,
        });
        GraphActivity {
            checked: panel.checked,
            run: panel.status.as_ref().map(|status| &status.run),
            permission: panel
                .status
                .as_ref()
                .and_then(|status| status.permission.as_ref())
                .map(|permission| tool_heading(&permission.tool_call, "Tool call")),
            assistant,
            tool,
            failed_tool,
        }
    }

    pub(in crate::workspace) fn permission_actions(
        &self,
        cx: &Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        let Some((project, workflow)) = self.state.selected else {
            return div();
        };
        let mut actions = div().flex().flex_wrap().gap_1().min_w_0();
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
                        .icon(IconName::ShieldAlert)
                        .label(format!("Node #{}", key.2))
                        .accessibility_label(format!("Review permission for node #{}", key.2))
                        .tooltip(format!("Permission needed for node #{}", key.2))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_permission(key, id, &permission, window, cx)
                        })),
                );
            }
        }
        div().flex_1().min_w_0().child(actions)
    }

    pub(super) fn render_follow_up(
        &self,
        key: NodeKey,
        panel: &RunPanel,
        run: i64,
        cx: &Context<Self>,
    ) -> Div {
        div()
            .flex()
            .items_center()
            .gap_2()
            .border_t_1()
            .border_color(cx.theme().border)
            .pt_2()
            .child(
                div().flex_1().min_w_0().child(
                    Textarea::new(&panel.follow_up)
                        .h(gpui_kit::rems(2.5))
                        .aria_label("Follow-up instructions"),
                ),
            )
            .child(
                Button::new("continue-run")
                    .outline()
                    .small()
                    .icon(IconName::Send)
                    .label("Continue session")
                    .disabled(panel.busy)
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.continue_node_run(key, run, cx)),
                    ),
            )
    }

    fn continue_node_run(&mut self, key: NodeKey, run: i64, cx: &mut Context<Self>) {
        if self
            .node_workspaces
            .get(&key)
            .is_some_and(|node| node.run.continue_target() == Some(run))
        {
            self.open_tab(key, super::Tab::Output, cx);
            self.start_node_run(key, Some(run), cx);
        }
    }

    pub(super) fn render_run_panel(
        &self,
        key: NodeKey,
        root: Div,
        window: &Window,
        cx: &Context<Self>,
    ) -> Div {
        let node = &self.node_workspaces[&key];
        let panel = &node.run;
        let status = panel.status.as_ref();
        // The inline composer needs a predictable amount of vertical room.
        // Below that, the node tab row carries one compact trigger and moves
        // the composer into a dialog instead of squeezing the transcript.
        let compact_follow_up = compact_follow_up(window);
        let running = status.is_some_and(|status| active(&status.run.state));
        let unsaved = node.draft.read(cx).value().as_ref() != node.saved;
        let mut toolbar = div()
            .flex()
            .flex_wrap()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(cx.theme().border)
            .pb_1();
        if let Some(status) = status.filter(|_| running) {
            let executable = status.run.executable.to_string_lossy();
            let name = status
                .run
                .executable
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(executable.as_ref());
            let (icon, color) = run_state_visual(&status.run.state, cx.theme());
            toolbar = toolbar
                .child(message(format!("Agent · {name}")).text_color(cx.theme().muted_foreground))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(Icon::new(icon).size_3().text_color(color))
                        .child(message(state_label(&status.run.state))),
                )
                .child(
                    message(format!("Run #{}", status.run.id))
                        .text_color(cx.theme().muted_foreground),
                );
        } else {
            toolbar = toolbar
                .child(
                    div().w_56().max_w_full().child(
                        Input::new(&panel.executable)
                            .small()
                            .aria_label("ACP executable"),
                    ),
                )
                .child(
                    Button::new("start-run")
                        .primary()
                        .small()
                        .icon(IconName::Play)
                        .label("Start run")
                        .disabled(panel.busy || unsaved || node.saving)
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.start_node_run(key, None, cx)),
                        ),
                );
        }
        if let Some(status) = status {
            let id = status.run.id;
            if running {
                toolbar = toolbar.child(
                    Button::new("cancel-run")
                        .outline()
                        .small()
                        .ml_auto()
                        .icon(IconName::Square)
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
                        .small()
                        .icon(IconName::RefreshCw)
                        .tooltip("Refresh run status and output")
                        .accessibility_label("Refresh run")
                        .on_click(cx.listener(move |this, _, _, cx| this.poll_node_run(key, cx))),
                );
            }
        }
        let mut summary = div().flex().items_center().flex_wrap().gap_2();
        if status.is_none() {
            summary = summary.child(message("A fresh run requires a saved assignment, a ready workspace, and an ACP executable."));
        }
        let mut follow_up = None;
        if unsaved {
            summary = summary.child(message(
                "Save assignment edits before starting a fresh run.",
            ));
        }
        if let Some(error) = &panel.error {
            summary = summary.child(message(error.clone()));
        }
        if let Some(status) = status {
            let id = status.run.id;
            let (state_icon, state_color) = run_state_visual(&status.run.state, cx.theme());
            if !running {
                summary = summary.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(Icon::new(state_icon).size_3().text_color(state_color))
                        .child(message(format!(
                            "Run #{id} · {}",
                            state_label(&status.run.state)
                        ))),
                );
            }
            match &status.run.snapshot {
                SnapshotState::Saved { .. } => {
                    summary = summary.child(
                        Button::new("review-run-snapshot")
                            .ghost()
                            .small()
                            .icon(IconName::GitCommitHorizontal)
                            .label("Review saved snapshot")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_review(key, Target::Run(id), cx)
                            })),
                    );
                }
                SnapshotState::Failed { detail } => {
                    summary = summary.child(message(format!("Snapshot failed. {detail}")))
                }
                _ if !running => summary = summary.child(message("No saved snapshot.")),
                _ => {}
            }
            if can_continue(status) && !compact_follow_up {
                follow_up = Some(self.render_follow_up(key, panel, id, cx));
            }
        }
        if let Some(notice) = panel.output_notice {
            summary = summary.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        Icon::new(IconName::CircleAlert)
                            .size_3()
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(message(notice).text_color(cx.theme().muted_foreground)),
            );
        }
        let workspace = cx.weak_entity();
        root.child(toolbar)
            .child(summary)
            .child(
                MessageScroller::new(
                    "run-output",
                    panel.output_scroll.clone(),
                    move |index, _, cx| {
                        let Some(entity) = workspace.upgrade() else {
                            return div();
                        };
                        let Some(node) = entity.read(cx).node_workspaces.get(&key) else {
                            return div();
                        };
                        let transcript = &node.run.output;
                        let Some(block) = transcript.blocks.get(index) else {
                            return div();
                        };
                        let replay = begins_replay(&transcript.blocks, index);
                        render_activity(block, replay, key, workspace.clone(), cx)
                    },
                )
                .with_jump_button_label("Jump to latest")
                .with_row_style(gpui_kit::StyleRefinement::default().pb_2().px_0())
                .flex_1()
                .min_h_0(),
            )
            .children(follow_up)
    }
}

fn begins_replay(blocks: &VecDeque<ActivityBlock>, index: usize) -> bool {
    blocks.get(index).is_some_and(|block| {
        block.origin == RunEventOrigin::Replay
            && (index == 0
                || blocks
                    .get(index - 1)
                    .is_none_or(|previous| previous.origin != RunEventOrigin::Replay))
    })
}

fn render_activity(
    block: &ActivityBlock,
    begins_replay: bool,
    key: NodeKey,
    workspace: WeakEntity<Workspace>,
    cx: &App,
) -> Div {
    let sequence = block.sequence;
    let row = div()
        .min_w_0()
        .max_w(gpui_kit::rems(52.))
        .px_2()
        .py_1()
        .when(begins_replay, |row| {
            row.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .pb_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(Icon::new(IconName::Clock).size_3())
                    .child("Replayed earlier output"),
            )
        });
    if let Activity::Text(Stream::Assistant, text) = &block.activity {
        return row.child(markdown_with_copy(
            format!("output-{key:?}-{sequence}"),
            text.clone(),
        ));
    }
    if let Activity::Text(Stream::Thought, text) = &block.activity {
        return row
            .flex()
            .flex_col()
            .items_start()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(Icon::new(IconName::Sparkles).size_3())
                    .child("Reasoning"),
            )
            .child(
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child(markdown_with_copy(
                        format!("reasoning-{key:?}-{sequence}"),
                        text.clone(),
                    )),
            );
    }
    let (title, summary, status, details) = match &block.activity {
        Activity::Text(_, _) => unreachable!("text activities return above"),
        Activity::Tool { id, fields } => {
            let title = tool_heading(fields, id);
            let status = fields
                .get("status")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            (title, tool_summary(fields), status, json_markdown(fields))
        }
        Activity::Other(update) => {
            let (title, summary) = other_heading(update);
            (title, summary, None, json_markdown(update))
        }
    };
    let toggle = Button::new(SharedString::from(format!("activity-{sequence}")))
        .ghost()
        .small()
        .icon(if block.expanded {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        })
        .label(title)
        .selected(block.expanded)
        .on_click(move |_, _, cx| {
            let _ = workspace.update(cx, |this, cx| {
                if let Some(node) = this.node_workspaces.get_mut(&key)
                    && let Some((index, block)) = node
                        .run
                        .output
                        .blocks
                        .iter_mut()
                        .enumerate()
                        .find(|(_, block)| block.sequence == sequence)
                {
                    block.expanded = !block.expanded;
                    node.run.output_scroll.update(cx, |scroll, cx| {
                        scroll.remeasure_items(index..index + 1, cx);
                    });
                    cx.notify();
                }
            });
        });
    let mut header = div()
        .flex()
        .items_center()
        .flex_wrap()
        .gap_2()
        .min_w_0()
        .child(toggle);
    if let Some(status) = status {
        header = header.child(
            div()
                .text_xs()
                .text_color(match status.as_str() {
                    "failed" | "error" => cx.theme().danger,
                    "pending" | "in_progress" | "running" => cx.theme().warning,
                    _ => cx.theme().muted_foreground,
                })
                .child(status.replace('_', " ")),
        );
    }
    if let Some(summary) = summary {
        header = header.child(
            div()
                .min_w_0()
                .truncate()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(summary),
        );
    }
    row.w_full()
        .border_t_1()
        .border_color(cx.theme().border)
        .flex()
        .flex_col()
        .items_start()
        .gap_1()
        .child(header)
        .when(block.expanded, |row| {
            row.child(markdown_with_copy(
                format!("details-{key:?}-{sequence}"),
                details,
            ))
        })
}

fn tool_heading(fields: &serde_json::Value, fallback: &str) -> String {
    fields
        .get("title")
        .and_then(serde_json::Value::as_str)
        .or_else(|| fields.get("kind").and_then(serde_json::Value::as_str))
        .unwrap_or(fallback)
        .to_owned()
}

fn tool_summary(fields: &serde_json::Value) -> Option<String> {
    if let Some(paths) = fields
        .get("locations")
        .and_then(serde_json::Value::as_array)
    {
        let paths = paths
            .iter()
            .filter_map(|location| location.get("path").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>();
        if let Some(first) = paths.first() {
            return Some(if paths.len() > 1 {
                format!("{first} +{}", paths.len() - 1)
            } else {
                (*first).to_owned()
            });
        }
    }
    if let Some(command) = fields
        .pointer("/rawInput/command")
        .and_then(serde_json::Value::as_str)
        .and_then(|command| command.lines().find(|line| !line.trim().is_empty()))
    {
        return Some(truncate_summary(command));
    }
    if let Some(kind) = fields.get("kind").and_then(serde_json::Value::as_str) {
        return Some(kind.to_owned());
    }
    if let Some(items) = fields.get("content").and_then(serde_json::Value::as_array) {
        let text = items.iter().find_map(|item| {
            item.pointer("/content/text")
                .and_then(serde_json::Value::as_str)
                .or_else(|| item.get("text").and_then(serde_json::Value::as_str))
        });
        if let Some(line) = text.and_then(|text| text.lines().find(|line| !line.trim().is_empty()))
        {
            return Some(truncate_summary(line));
        }
        if !items.is_empty() {
            return Some(format!(
                "{} content {}",
                items.len(),
                if items.len() == 1 { "item" } else { "items" }
            ));
        }
    }
    None
}

fn truncate_summary(text: &str) -> String {
    const LIMIT: usize = 120;
    if text.chars().count() <= LIMIT {
        return text.trim().to_owned();
    }
    let mut summary = text.chars().take(LIMIT).collect::<String>();
    summary.push('…');
    summary
}

fn other_heading(update: &serde_json::Value) -> (String, Option<String>) {
    let name = update
        .get("sessionUpdate")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("Unrecognized update");
    match name {
        "available_commands_update" => {
            let count = update
                .get("availableCommands")
                .and_then(serde_json::Value::as_array)
                .map_or(0, Vec::len);
            (
                "Available commands".into(),
                Some(format!(
                    "{count} {}",
                    if count == 1 { "command" } else { "commands" }
                )),
            )
        }
        "usage_update" => {
            let used = update.get("used").and_then(serde_json::Value::as_u64);
            let size = update.get("size").and_then(serde_json::Value::as_u64);
            (
                "Usage updated".into(),
                match (used, size) {
                    (Some(used), Some(size)) => Some(format!("{used} of {size} tokens")),
                    (Some(used), None) => Some(format!("{used} tokens used")),
                    _ => None,
                },
            )
        }
        "session_info_update" => {
            let title = update
                .get("title")
                .and_then(serde_json::Value::as_str)
                .filter(|title| !title.trim().is_empty());
            ("Session details updated".into(), title.map(str::to_owned))
        }
        _ => (name.replace('_', " "), None),
    }
}

fn json_markdown(value: &serde_json::Value) -> String {
    let details = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
    fenced_code("json", &details)
}

fn fenced_code(language: &str, code: &str) -> String {
    let mut fence = 3;
    let mut run = 0;
    for character in code.chars() {
        if character == '`' {
            run += 1;
            fence = fence.max(run + 1);
        } else {
            run = 0;
        }
    }
    let fence = "`".repeat(fence);
    format!(
        "{fence}{language}\n{}\n{fence}",
        code.trim_end_matches('\n')
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn chunk(text: &str) -> serde_json::Value {
        json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}})
    }

    #[test]
    fn assembles_streams_and_updates_tool_in_place() {
        let mut transcript = Transcript::default();
        transcript.push(1, RunEventOrigin::Replay, chunk("Earlier"));
        transcript.push(2, RunEventOrigin::Live, chunk("```rust\n"));
        transcript.push(3, RunEventOrigin::Live, chunk("let café = 1;\n```"));
        transcript.push(4, RunEventOrigin::Live, json!({
            "sessionUpdate": "tool_call", "toolCallId": "read", "title": "Read source", "status": "pending"
        }));
        transcript.push(
            5,
            RunEventOrigin::Live,
            json!({
                "sessionUpdate": "agent_message_chunk", "content": {"type": "unknown"}
            }),
        );
        transcript.push(
            6,
            RunEventOrigin::Live,
            json!({
                "sessionUpdate": "tool_call_update", "toolCallId": "read", "status": "completed"
            }),
        );
        assert_eq!(transcript.blocks.len(), 4);
        assert!(
            matches!(&transcript.blocks[1].activity, Activity::Text(Stream::Assistant, text)
            if text == "```rust\nlet café = 1;\n```")
        );
        assert!(
            matches!(&transcript.blocks[2].activity, Activity::Tool { fields, .. }
            if fields["title"] == "Read source" && fields["status"] == "completed")
        );
        assert!(matches!(&transcript.blocks[3].activity, Activity::Other(_)));
    }

    #[test]
    fn tool_summaries_prefer_locations_commands_and_text_content() {
        assert_eq!(
            tool_summary(&json!({
                "locations": [{"path": "src/main.rs"}, {"path": "src/lib.rs"}]
            })),
            Some("src/main.rs +1".into())
        );
        assert_eq!(
            tool_summary(&json!({"rawInput": {"command": "  \nprintf hello\nnext"}})),
            Some("printf hello".into())
        );
        assert_eq!(
            tool_summary(&json!({"content": [{"content": {"text": "\n  Result line  "}}]})),
            Some("Result line".into())
        );
        assert_eq!(
            tool_summary(&json!({"content": [{}, {}]})),
            Some("2 content items".into())
        );
        assert_eq!(
            tool_summary(&json!({"content": [{}]})),
            Some("1 content item".into())
        );
        assert_eq!(tool_summary(&json!({})), None);
    }

    #[test]
    fn summaries_truncate_on_character_boundaries() {
        let summary = truncate_summary(&"x".repeat(200));
        assert_eq!(summary.chars().count(), 121);
        assert!(summary.ends_with('…'));

        let unicode = "é".repeat(121);
        let summary = truncate_summary(&unicode);
        assert_eq!(summary.chars().count(), 121);
        assert_eq!(summary.chars().last(), Some('…'));
        assert!(summary.starts_with("éé"));
    }

    #[test]
    fn fenced_code_widens_the_fence_past_inner_backtick_runs() {
        assert_eq!(
            fenced_code("json", "{\"ok\": true}\n"),
            "```json\n{\"ok\": true}\n```"
        );
        let nested = fenced_code("text", "```\ninner\n```");
        assert!(nested.starts_with("````text\n"), "{nested}");
        assert!(nested.ends_with("\n````"), "{nested}");
    }

    #[test]
    fn streaming_chunks_merge_across_a_partial_code_fence() {
        let mut transcript = Transcript::default();
        transcript.push(
            1,
            RunEventOrigin::Live,
            chunk("Here is the change:\n\n```rust\nfn main() {"),
        );
        transcript.push(2, RunEventOrigin::Live, chunk("\n}\n```\n\nDone."));
        assert_eq!(transcript.blocks.len(), 1);
        assert!(
            matches!(&transcript.blocks[0].activity, Activity::Text(Stream::Assistant, text)
            if text == "Here is the change:\n\n```rust\nfn main() {\n}\n```\n\nDone.")
        );
    }

    #[test]
    fn replay_label_starts_only_at_a_replay_boundary() {
        fn block(sequence: u64, origin: RunEventOrigin) -> ActivityBlock {
            ActivityBlock {
                sequence,
                origin,
                activity: Activity::Other(json!({})),
                expanded: false,
            }
        }

        let blocks = VecDeque::from([
            block(1, RunEventOrigin::Replay),
            block(2, RunEventOrigin::Replay),
            block(3, RunEventOrigin::Live),
            block(4, RunEventOrigin::Replay),
        ]);
        assert!(begins_replay(&blocks, 0));
        assert!(!begins_replay(&blocks, 1));
        assert!(!begins_replay(&blocks, 2));
        assert!(begins_replay(&blocks, 3));
        assert!(!begins_replay(&blocks, 4));
    }

    #[test]
    fn trims_whole_blocks_and_caps_oversized_unicode() {
        let mut transcript = Transcript::default();
        transcript.push(1, RunEventOrigin::Replay, chunk("Earlier"));
        transcript.push(2, RunEventOrigin::Live, chunk(&"猫".repeat(OUTPUT_LIMIT)));
        assert!(transcript.trimmed);
        assert_eq!(transcript.blocks.len(), 1);
        assert!(
            matches!(&transcript.blocks[0].activity, Activity::Text(_, text)
            if text.len() <= OUTPUT_LIMIT && text.ends_with('猫'))
        );
        transcript.push(3, RunEventOrigin::Live, chunk("New output"));
        assert!(
            matches!(&transcript.blocks.back().expect("new output block").activity,
            Activity::Text(_, text) if text == "New output")
        );
    }
    #[gpui_kit::test]
    fn incoming_events_preserve_the_scrolled_reading_anchor(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::{IntoElement, Render, ScrollDelta, ScrollEvent, point, px};

        struct OutputHarness {
            scroll: Entity<MessageScrollerState>,
        }

        impl Render for OutputHarness {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().size(px(400.)).child(MessageScroller::new(
                    "test-output",
                    self.scroll.clone(),
                    |_, _, _| div().h(px(40.)),
                ))
            }
        }

        fn tool(sequence: u64, id: u64, kind: &str) -> RunEvent {
            serde_json::from_value(json!({
                "sequence": sequence,
                "origin": "live",
                "update": {"sessionUpdate": kind, "toolCallId": id.to_string(), "status": "completed"}
            })).expect("event fixture matches the backend schema")
        }

        cx.update(gpui_kit::init);
        let scroll = cx.new(|cx| MessageScrollerState::new(0, cx));
        let (_, cx) = cx.add_window_view({
            let scroll = scroll.clone();
            move |_, _| OutputHarness { scroll }
        });
        let mut transcript = Transcript::default();
        cx.update(|window, cx| {
            transcript.apply_events(
                (1..=30).map(|id| tool(id, id, "tool_call")).collect(),
                false,
                &scroll,
                cx,
            );
            let _ = window.draw(cx);
        });
        let tail = cx.update(|_, cx| scroll.read(cx).logical_scroll_top());
        cx.simulate_event(ScrollEvent {
            position: point(px(200.), px(200.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(360.))),
            ..Default::default()
        });
        let reading = cx.update(|_, cx| {
            assert!(scroll.read(cx).is_scrolled_up());
            scroll.read(cx).logical_scroll_top()
        });
        assert!(reading.item_ix < tail.item_ix);

        cx.update(|window, cx| {
            transcript.apply_events(
                vec![tool(31, 1, "tool_call_update"), tool(32, 32, "tool_call")],
                false,
                &scroll,
                cx,
            );
            let _ = window.draw(cx);
            assert_eq!(scroll.read(cx).item_count(), 31);
            assert_eq!(scroll.read(cx).logical_scroll_top(), reading);
            scroll.update(cx, |scroll, cx| scroll.scroll_to_end(cx));
            let _ = window.draw(cx);
            assert!(scroll.read(cx).is_following_tail());
        });
    }
    #[gpui_kit::test]
    fn many_permission_requests_keep_the_workflow_controls_reachable(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use super::super::{Load, Tab};
        use crate::workspace::workspace_tests::{open_workspace, workflow};
        use gpui_kit::test::TestWindowExt as _;

        let (handle, workspace) = open_workspace(cx);
        cx.simulate_window_resize(
            handle,
            gpui_kit::size(gpui_kit::px(720.), gpui_kit::px(480.)),
        );
        cx.update_window(handle, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.graph = Load::Ready(
                    serde_json::from_value(json!({
                        "workflow": workflow("project-a", 7),
                        "nodes": (1..=6).map(|id| json!({
                            "id": id, "workflow_id": 7, "prompt": format!("Permission node {id}")
                        })).collect::<Vec<_>>(),
                        "connections": []
                    }))
                    .expect("workflow graph fixture matches the storage schema"),
                );
                for id in 1..=6 {
                    workspace.open_node(id, window, cx);
                    let panel = &mut workspace
                        .node_workspaces
                        .get_mut(&(1, 7, id))
                        .expect("node workspace")
                        .run;
                    panel.status = Some(
                        serde_json::from_value(json!({
                            "run": {
                                "id": 100 + id, "workflow_id": 7, "node_id": id,
                                "prompt": format!("Permission node {id}"),
                                "directory": "/tmp/fixture", "change_id": "fixture",
                                "executable": "/tmp/fixture/agent",
                                "state": {"status": "awaiting_permission"},
                                "snapshot": {"status": "not_recorded"}
                            },
                            "permission": {
                                "id": id,
                                "tool_call": {"title": format!("Write file {id}")},
                                "options": [{"option_id": "deny", "name": "Deny once", "kind": "reject_once"}]
                            }
                        }))
                        .expect("run fixture matches the backend schema"),
                    );
                }
                let node = workspace.node_workspaces.get_mut(&(1, 7, 1)).expect("node");
                node.tab = Tab::Assignment;
                cx.notify();
            });
            window.render_frame(cx);
            let viewport = window.viewport_size();
            let zoom = window.find("graph-zoom-reset").bounds();
            assert!(
                zoom.right() <= viewport.width,
                "graph zoom {zoom:?} is outside {viewport:?}"
            );
            for id in 1..=6 {
                let button = window.find(format!("permission-{}", 100 + id));
                assert!(button.visible(), "permission {id} is hidden");
                assert!(
                    button.bounds().bottom() <= viewport.height,
                    "permission {id} is outside {viewport:?}"
                );
            }
            window.click("permission-102", cx);
            assert!(window.find("dismiss-permission").visible());
            window.press("escape", cx);
            assert!(window.try_find("dismiss-permission").is_none());
        })
        .expect("workspace window remains open");
    }

    #[gpui_kit::test]
    fn permission_dialog_keeps_the_pinned_diff_at_the_same_line(cx: &mut gpui_kit::TestAppContext) {
        use std::fmt::Write as _;

        use super::super::{Load, Review, Tab};
        use crate::workspace::workspace_tests::{open_workspace, workflow};
        use gpui_kit::component::diff::{DiffFile, DiffLinePosition, DiffSide, DiffState};
        use gpui_kit::test::TestWindowExt as _;

        let (handle, workspace) = open_workspace(cx);
        cx.simulate_window_resize(
            handle,
            gpui_kit::size(gpui_kit::px(720.), gpui_kit::px(520.)),
        );
        cx.update_window(handle, |_, window, cx| {
            let diff = workspace.update(cx, |workspace, cx| {
                workspace.graph = Load::Ready(serde_json::from_value(json!({
                    "workflow": workflow("project-a", 7),
                    "nodes": [
                        {"id": 1, "workflow_id": 7, "prompt": "Review"},
                        {"id": 2, "workflow_id": 7, "prompt": "Request permission"}
                    ],
                    "connections": []
                })).expect("workflow graph fixture matches the storage schema"));
                workspace.open_node(2, window, cx);
                workspace.open_node(1, window, cx);
                workspace.expanded_workbench = true;

                workspace.node_workspaces.get_mut(&(1, 7, 2)).expect("permission node").run.status =
                    Some(serde_json::from_value(json!({
                        "run": {
                            "id": 42, "workflow_id": 7, "node_id": 2, "prompt": "Request permission",
                            "directory": "/tmp/fixture", "change_id": "fixture", "executable": "/tmp/fixture/agent",
                            "state": {"status": "awaiting_permission"},
                            "snapshot": {"status": "not_recorded"}
                        },
                        "permission": {
                            "id": 1, "tool_call": {"title": "Write fixture"},
                            "options": [{"option_id": "deny", "name": "Deny once", "kind": "reject_once"}]
                        }
                    })).expect("run fixture matches the backend schema"));

                let path = SharedString::from("src/review.rs");
                let mut patch = String::from("diff --git a/src/review.rs b/src/review.rs\nnew file mode 100644\n--- /dev/null\n+++ b/src/review.rs\n@@ -0,0 +1,250 @@\n");
                for line in 1..=250 {
                    writeln!(&mut patch, "+// fixture line {line}").expect("write patch line");
                }
                let files = DiffFile::parse(&patch).expect("fixture patch is valid");
                let diff = cx.new(|cx| DiffState::new(files, cx));
                let node = workspace.node_workspaces.get_mut(&(1, 7, 1)).expect("review node");
                node.tab = Tab::Changes;
                node.target = Target::Run(41);
                node.review = Load::Ready(Review {
                    label: "Run #41 snapshot".into(),
                    commit: "pinned".into(),
                    conflicted: false,
                    files: vec![(path, diff.clone())],
                    selected: 0,
                });
                cx.notify();
                diff
            });
            window.render_frame(cx);
            diff.update(cx, |diff, cx| {
                diff.scroll_to_line(DiffLinePosition::new("src/review.rs", DiffSide::Modified, 120), cx);
            });
            window.render_frame(cx);
            let line = ("line", 119_usize);
            let before = window.find(line).bounds();
            assert!(window.find(line).visible(), "the diff scrolled to the deep line");

            window.click(line, cx);
            let prior_focus = window.focused(cx).expect("diff has keyboard focus");
            window.click("permission-42", cx);
            assert!(window.find("dismiss-permission").visible());
            window.press("escape", cx);
            assert!(window.try_find("dismiss-permission").is_none());
            assert!(prior_focus.is_focused(window), "dismissing restores the review focus");

            assert_eq!(window.find(line).bounds(), before);
            let selected = &workspace.read(cx).node_workspaces[&(1, 7, 1)];
            assert!(matches!(selected.target, Target::Run(41)));
            let Load::Ready(review) = &selected.review else {
                panic!("pinned review remains ready");
            };
            assert_eq!(review.files[review.selected].1, diff);
        }).expect("workspace window remains open");
    }

    #[gpui_kit::test]
    fn follow_up_controls_stay_inside_the_window(cx: &mut gpui_kit::TestAppContext) {
        use super::super::{Load, Tab};
        use crate::workspace::workspace_tests::{open_workspace, workflow};
        use gpui_kit::test::TestWindowExt as _;

        let (handle, workspace) = open_workspace(cx);
        cx.update_window(handle, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.graph = Load::Ready(
                    serde_json::from_value(json!({
                        "workflow": workflow("project-a", 7),
                        "nodes": [{"id": 1, "workflow_id": 7, "prompt": "Review"}],
                        "connections": []
                    }))
                    .expect("workflow graph fixture matches the storage schema"),
                );
                workspace.open_node(1, window, cx);
                workspace.expanded_workbench = false;
                let node = workspace.node_workspaces.get_mut(&(1, 7, 1)).expect("node");
                node.tab = Tab::Output;
                node.run.status = Some(
                    serde_json::from_value(json!({
                        "run": {
                            "id": 42, "workflow_id": 7, "node_id": 1, "prompt": "Review",
                            "directory": "/tmp/fixture", "change_id": "fixture",
                            "executable": "/tmp/fixture/agent", "session_id": "session",
                            "state": {"status": "completed"},
                            "snapshot": {"status": "not_recorded"}
                        },
                        "permission": null
                    }))
                    .expect("run fixture matches the backend schema"),
                );
                node.run.busy = false;
                cx.notify();
            });
            window.render_frame(cx);
            assert!(window.find("continue-run").visible());
        })
        .expect("workspace window remains open");

        cx.simulate_window_resize(
            handle,
            gpui_kit::size(gpui_kit::px(720.), gpui_kit::px(480.)),
        );
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let window_bounds = window.viewport_size();
            let compact = window.find("continue-session").bounds();
            assert!(
                compact.bottom() <= window_bounds.height,
                "compact follow-up trigger {compact:?} exceeds viewport {window_bounds:?}"
            );
            assert!(window.try_find("continue-run").is_none());
            let inputs_before = window.find_all("input-state").len();
            window.click("continue-session", cx);
            assert!(window.find("submit-follow-up").visible());
            assert!(window.find("cancel-follow-up").visible());
            assert!(
                window.find_all("input-state").len() > inputs_before,
                "the follow-up dialog contains an instructions field"
            );
            window.press("escape", cx);
            assert!(window.try_find("submit-follow-up").is_none());

            gpui_kit::component::Theme::global_mut(cx).font_size *= 1.5;
            window.render_frame(cx);
            let zoomed = window.find("continue-session").bounds();
            assert!(
                zoomed.bottom() <= window_bounds.height,
                "zoomed follow-up trigger {zoomed:?} exceeds viewport {window_bounds:?}"
            );
            window.click("continue-session", cx);
            assert!(window.find("submit-follow-up").visible());
            assert!(window.find("cancel-follow-up").visible());
            window.press("escape", cx);
            assert!(window.try_find("submit-follow-up").is_none());
        })
        .expect("workspace window remains open");
    }
}
