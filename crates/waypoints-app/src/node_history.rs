use gpui_kit::assets::IconName;
use gpui_kit::base::{Disableable as _, StyledExt as _, TestSupportExt as _};
use gpui_kit::component::{
    ActiveTheme as _, Icon, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    scroll::ScrollableElement as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, ClipboardItem, Context, Div, InteractiveElement as _, IntoElement as _,
    ListState, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    WeakEntity, div, list, relative,
};
use waypoints_backend::{Client, RunStatus};
use waypoints_core::{
    run::SnapshotState,
    storage::{NodeRevisionEntry, NodeRevisionPage},
};

use super::{
    Load, NodeKey, NodeWorkspace, Tab, Target, Workspace, message, node_runs, run_backend,
    run_state_visual, short_id,
};

const HISTORY_PAGE_SIZE: usize = 30;

type History = Load<(Vec<RunStatus>, Result<NodeRevisionPage, String>)>;

#[derive(Clone, PartialEq, Eq)]
pub(super) enum HistorySelection {
    Run(i64),
    Revision(String),
}

enum HistoryDetail<'a> {
    Run(&'a RunStatus),
    Revision(&'a NodeRevisionEntry),
}

impl HistoryDetail<'_> {
    fn selection(&self) -> HistorySelection {
        match self {
            Self::Run(status) => HistorySelection::Run(status.run.id),
            Self::Revision(entry) => {
                HistorySelection::Revision(entry.evolution.revision.commit_id.clone())
            }
        }
    }
}

fn history_detail(node: &NodeWorkspace) -> Option<HistoryDetail<'_>> {
    let Load::Ready((runs, page)) = &node.history else {
        return None;
    };
    let entries = page
        .as_ref()
        .ok()
        .map_or(&[][..], |page| page.entries.as_slice());
    let selected = match node.history_selection.as_ref() {
        Some(HistorySelection::Run(id)) => runs
            .iter()
            .find(|status| status.run.id == *id)
            .map(HistoryDetail::Run),
        Some(HistorySelection::Revision(commit)) => entries
            .iter()
            .find(|entry| &entry.evolution.revision.commit_id == commit)
            .map(HistoryDetail::Revision),
        None => None,
    };
    selected
        .or_else(|| runs.first().map(HistoryDetail::Run))
        .or_else(|| entries.first().map(HistoryDetail::Revision))
}

fn readable_timestamp(timestamp: &str) -> String {
    let timestamp = timestamp.trim();
    let Some((date, time)) = timestamp.split_once('T') else {
        return timestamp.to_owned();
    };
    let time = time.trim_end_matches('Z');
    let clock = time.get(..5).unwrap_or(time);
    let zone = if timestamp.ends_with('Z') {
        Some("UTC")
    } else {
        time.rfind(['+', '-']).map(|offset| &time[offset..])
    };
    match zone {
        Some(zone) => format!("{date} {clock} {zone}"),
        None => format!("{date} {clock}"),
    }
}

fn describe_operation(operation: &str) -> String {
    let operation = operation.trim();
    if operation.is_empty() {
        return String::new();
    }
    let mut characters = operation.chars();
    let Some(first) = characters.next() else {
        return String::new();
    };
    first.to_uppercase().collect::<String>() + characters.as_str()
}

fn history_row_count(history: &History) -> usize {
    match history {
        Load::Ready((runs, revisions)) => {
            2 + runs.len().max(1)
                + revisions
                    .as_ref()
                    .map_or(1, |page| page.entries.len().max(1))
        }
        _ => 1,
    }
}

fn append_older_page(
    page: &mut NodeRevisionPage,
    older: NodeRevisionPage,
    scroll: &ListState,
) -> Result<(), String> {
    if page.workflow_id != older.workflow_id
        || page.node_id != older.node_id
        || page.change_id != older.change_id
        || page.native_operation != older.native_operation
    {
        return Err(
            "History changed while loading older revisions. Refresh history to retry.".into(),
        );
    }
    let end = scroll.item_count();
    let empty_row = usize::from(page.entries.is_empty() && !older.entries.is_empty());
    let count = older.entries.len();
    page.entries.extend(older.entries);
    page.next_cursor = older.next_cursor;
    scroll.splice(end - empty_row..end, count);
    Ok(())
}

impl NodeWorkspace {
    fn begin_history_refresh(&mut self) -> Option<u64> {
        if matches!(self.history, Load::Loading) {
            return None;
        }
        self.history = Load::Loading;
        self.history_scroll.reset(1);
        self.history_stale = false;
        self.loading_older = false;
        self.older_error = None;
        self.history_generation += 1;
        Some(self.history_generation)
    }

    fn finish_history_refresh(&mut self, generation: u64, history: History) -> bool {
        if self.history_generation != generation {
            return false;
        }
        self.history = history;
        self.history_scroll.reset(history_row_count(&self.history));
        true
    }

    fn begin_older_page(&mut self) -> Option<(u64, String)> {
        let Load::Ready((_, Ok(page))) = &self.history else {
            return None;
        };
        let cursor = page.next_cursor.clone().filter(|_| !self.loading_older)?;
        self.loading_older = true;
        self.older_error = None;
        Some((self.history_generation, cursor))
    }

    fn finish_older_page(
        &mut self,
        generation: u64,
        result: Result<NodeRevisionPage, String>,
    ) -> bool {
        if self.history_generation != generation || !self.loading_older {
            return false;
        }
        let Load::Ready((_, Ok(page))) = &mut self.history else {
            return false;
        };
        self.loading_older = false;
        self.older_error = result
            .and_then(|older| append_older_page(page, older, &self.history_scroll))
            .err();
        true
    }

    #[cfg(feature = "screenshot-fixtures")]
    pub(super) fn install_screenshot_history(&mut self) {
        use serde_json::json;

        fn run(id: i64, state: serde_json::Value, snapshot: serde_json::Value) -> RunStatus {
            serde_json::from_value(json!({
                "run": {
                    "id": id,
                    "workflow_id": 7,
                    "node_id": 1,
                    "prompt": "Polish the node workspace.",
                    "directory": "/tmp/waypoints-screenshots/workspace",
                    "change_id": "screenshot-change",
                    "executable": "/usr/local/bin/codex-acp",
                    "session_id": "screenshot-session",
                    "continued_from": null,
                    "process_id": null,
                    "state": state,
                    "snapshot": snapshot
                },
                "permission": null
            }))
            .expect("screenshot run fixture matches the backend schema")
        }

        let saved_snapshot = json!({
            "status": "saved",
            "revision": {
                "change_id": "pqrstuvwxyzabcdefghijklmnop",
                "commit_id": "0123456789abcdef0123456789abcdef01234567",
                "description": "Polish the node workspace",
                "conflicted": false
            }
        });
        let runs = vec![
            run(42, json!({"status": "completed"}), saved_snapshot),
            run(
                41,
                json!({"status": "cancelled"}),
                json!({"status": "not_recorded"}),
            ),
            run(
                40,
                json!({"status": "failed", "detail": "Adapter exited before completing the turn."}),
                json!({"status": "failed", "detail": "Snapshot could not be confirmed before the deadline."}),
            ),
        ];
        let page: NodeRevisionPage = serde_json::from_value(json!({
            "workflow_id": 7,
            "node_id": 1,
            "change_id": "screenshot-change",
            "native_operation": "operation-17",
            "current_commit_ids": ["0123456789abcdef0123456789abcdef01234567"],
            "entries": (0..12).map(|index| json!({
                "evolution": {
                    "revision": {
                        "change_id": "pqrstuvwxyzabcdefghijklmnop",
                        "commit_id": format!("0123456789ab{index:028x}"),
                        "description": if index == 0 {
                            "Polish the node workspace".to_owned()
                        } else {
                            format!("Follow-up revision {index}")
                        },
                        "conflicted": index == 3
                    },
                    "predecessor_commit_ids": [],
                    "operation_id": "operation-17",
                    "operation_description": if index == 0 {
                        "snapshot working copy".to_owned()
                    } else {
                        format!("describe commit {}", "0123456789abcdef0123456789abcdef01234567")
                    },
                    "timestamp": format!("2026-11-04T{:02}:{:02}:00-08:00", 12 + index / 4, 10 + index % 4 * 7)
                },
                "is_current": index == 0,
                "run_ids": if index == 0 { vec![42] } else if index == 2 { vec![41] } else { vec![] }
            })).collect::<Vec<_>>(),
            "next_cursor": "older-revisions-cursor"
        }))
        .expect("screenshot revision page fixture matches the storage schema");
        self.history = Load::Ready((runs, Ok(page)));
        self.history_stale = false;
        self.loading_older = false;
        self.older_error = None;
        self.history_scroll.reset(history_row_count(&self.history));
    }
}

impl Workspace {
    pub(super) fn load_history(&mut self, key: NodeKey, cx: &mut Context<Self>) {
        let Some(node) = self.node_workspaces.get_mut(&key) else {
            return;
        };
        let Some(generation) = node.begin_history_refresh() else {
            return;
        };
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    run_backend(|client: Client| async move {
                        let runs = client
                            .node_runs(key.1, key.2)
                            .await
                            .map_err(|error| error.to_string())?;
                        let revisions = client
                            .node_revisions(key.1, key.2, HISTORY_PAGE_SIZE, None)
                            .await
                            .map_err(|error| error.to_string());
                        Ok((runs, revisions))
                    })
                })
                .await;
            let history = match result {
                Ok(history) => Load::Ready(history),
                Err(error) => Load::Error(error),
            };
            let _ = this.update(cx, |this, cx| {
                if let Some(node) = this.node_workspaces.get_mut(&key)
                    && node.finish_history_refresh(generation, history)
                {
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn load_older_revisions(&mut self, key: NodeKey, cx: &mut Context<Self>) {
        let Some(node) = self.node_workspaces.get_mut(&key) else {
            return;
        };
        let Some((generation, cursor)) = node.begin_older_page() else {
            return;
        };
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    run_backend(async |client: Client| {
                        client
                            .node_revisions(key.1, key.2, HISTORY_PAGE_SIZE, Some(&cursor))
                            .await
                            .map_err(|error| error.to_string())
                    })
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Some(node) = this.node_workspaces.get_mut(&key)
                    && node.finish_older_page(generation, result)
                {
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn render_node_history(
        &self,
        key: NodeKey,
        node: &NodeWorkspace,
        root: Div,
        cx: &Context<Self>,
    ) -> Div {
        let page = match &node.history {
            Load::Ready((_, Ok(page))) => Some(page),
            _ => None,
        };
        let workspace = cx.weak_entity();
        let detail = history_detail(node);
        let selected = detail.as_ref().map(HistoryDetail::selection);
        let list_workspace = workspace.clone();
        root.child(
            div()
                .flex()
                .items_center()
                .flex_wrap()
                .gap_2()
                .child(
                    Button::new("reload-history")
                        .ghost()
                        .small()
                        .icon(IconName::RefreshCw)
                        .tooltip("Refresh run and revision history")
                        .accessibility_label("Refresh history")
                        .disabled(matches!(node.history, Load::Loading))
                        .on_click(cx.listener(move |this, _, _, cx| this.load_history(key, cx))),
                )
                .when_some(page, |toolbar, page| {
                    toolbar
                        .when(page.next_cursor.is_some(), |toolbar| {
                            toolbar.child(
                                Button::new("older-revisions")
                                    .ghost()
                                    .small()
                                    .icon(IconName::ChevronsDown)
                                    .label(if node.loading_older {
                                        "Loading older revisions…"
                                    } else {
                                        "Load older revisions"
                                    })
                                    .disabled(node.loading_older)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.load_older_revisions(key, cx)
                                    })),
                            )
                        })
                        .child(
                            message(if page.next_cursor.is_some() {
                                format!("{} revisions loaded", page.entries.len())
                            } else {
                                format!("All {} revisions loaded", page.entries.len())
                            })
                            .text_xs()
                            .text_color(cx.theme().muted_foreground),
                        )
                        .when(node.history_stale, |toolbar| {
                            toolbar.child(
                                message("New history is available. Refresh to update.")
                                    .id("history-stale")
                                    .test_support()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground),
                            )
                        })
                }),
        )
        .when_some(node.older_error.clone(), |root, error| {
            root.child(message(error).text_color(cx.theme().danger))
        })
        .child(
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .child(
                    div()
                        .id("history-navigation")
                        .test_support()
                        .flex()
                        .flex_col()
                        .w_72()
                        .max_w(relative(0.42))
                        .min_h_0()
                        .border_r_1()
                        .border_color(cx.theme().border)
                        .pr_2()
                        .child(
                            list(node.history_scroll.clone(), move |index, _, cx| {
                                let Some(entity) = list_workspace.upgrade() else {
                                    return div().into_any_element();
                                };
                                let Some(node) = entity.read(cx).node_workspaces.get(&key) else {
                                    return div().into_any_element();
                                };
                                history_row(
                                    index,
                                    &node.history,
                                    selected.as_ref(),
                                    key,
                                    list_workspace.clone(),
                                    cx,
                                )
                                .into_any_element()
                            })
                            .flex_1()
                            .min_h_0(),
                        )
                        .vertical_scrollbar(&node.history_scroll),
                )
                .child(
                    div()
                        .id("history-detail")
                        .test_support()
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .p_2()
                        .overflow_y_scroll()
                        .track_scroll(&node.history_detail_scroll)
                        .when_some(detail, |panel, detail| match detail {
                            HistoryDetail::Run(status) => panel
                                .child(run_row(status, key, workspace.clone(), cx))
                                .child(
                                    div()
                                        .pt_3()
                                        .text_sm()
                                        .font_semibold()
                                        .child("Run instructions"),
                                )
                                .child(
                                    div()
                                        .pt_1()
                                        .text_sm()
                                        .whitespace_normal()
                                        .child(status.run.prompt.clone()),
                                )
                                .child(self.render_history_run_output(key, status.run.id, cx)),
                            HistoryDetail::Revision(entry) => {
                                panel.child(revision_row(entry, key, workspace, cx))
                            }
                        }),
                ),
        )
    }

    fn render_history_run_output(&self, key: NodeKey, run: i64, cx: &Context<Self>) -> Div {
        let activity = self.graph_activity(key);
        let retained = activity.run.is_some_and(|status| status.id == run)
            && (activity.assistant.is_some() || activity.tool.is_some());
        let muted = cx.theme().muted_foreground;
        let section = div()
            .mt_3()
            .pt_2()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(div().text_sm().font_semibold().child("Run output"));
        if !retained {
            return section.child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child("No retained output is available for this run."),
            );
        }
        section
            .when_some(activity.assistant, |section, text| {
                section.child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .line_clamp(4)
                        .child(text.to_owned()),
                )
            })
            .when_some(activity.exception().or(activity.tool), |section, title| {
                section.child(div().text_xs().text_color(muted).line_clamp(2).child(title))
            })
            .child(
                Button::new("history-open-output")
                    .ghost()
                    .small()
                    .label("Open output")
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.open_tab(key, Tab::Output, cx)),
                    ),
            )
    }
}

fn history_row(
    index: usize,
    history: &History,
    selected: Option<&HistorySelection>,
    key: NodeKey,
    workspace: WeakEntity<Workspace>,
    cx: &App,
) -> Div {
    let (runs, revisions) = match history {
        Load::Idle | Load::Loading => return message("Loading history…"),
        Load::Error(error) => return message(error.clone()).text_color(cx.theme().danger),
        Load::Ready((runs, revisions)) => (runs, revisions),
    };
    if index == 0 {
        return div().py_1().px_2().text_sm().font_semibold().child("Runs");
    }
    let revision_heading = runs.len().max(1) + 1;
    if index < revision_heading {
        return runs.get(index - 1).map_or_else(
            || message("No runs yet.").text_color(cx.theme().muted_foreground),
            |run| history_choice(HistoryDetail::Run(run), selected, key, workspace, cx),
        );
    }
    if index == revision_heading {
        return div()
            .pt_3()
            .pb_1()
            .px_2()
            .text_sm()
            .font_semibold()
            .child("Native revisions");
    }
    match revisions {
        Ok(page) => page.entries.get(index - revision_heading - 1).map_or_else(
            || message("No native revisions.").text_color(cx.theme().muted_foreground),
            |entry| history_choice(HistoryDetail::Revision(entry), selected, key, workspace, cx),
        ),
        Err(error) => message(error.clone()).text_color(cx.theme().danger),
    }
}

fn history_choice(
    detail: HistoryDetail<'_>,
    selected: Option<&HistorySelection>,
    key: NodeKey,
    workspace: WeakEntity<Workspace>,
    cx: &App,
) -> Div {
    let selection = detail.selection();
    let (id, icon, color, title, subtitle) = match detail {
        HistoryDetail::Run(status) => {
            let (icon, color) = run_state_visual(&status.run.state, cx.theme());
            (
                format!("history-run-{}", status.run.id),
                icon,
                color,
                super::node_title(status.run.id, &status.run.prompt),
                format!(
                    "Run #{} · {}",
                    status.run.id,
                    node_runs::state_label(&status.run.state)
                ),
            )
        }
        HistoryDetail::Revision(entry) => {
            let revision = &entry.evolution.revision;
            let title = revision
                .description
                .lines()
                .find(|line| !line.trim().is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| "Native revision".into());
            (
                format!("history-revision-{}", revision.commit_id),
                IconName::GitCommitHorizontal,
                cx.theme().muted_foreground,
                title,
                format!(
                    "{} · {}",
                    short_id(&revision.commit_id),
                    readable_timestamp(&entry.evolution.timestamp)
                ),
            )
        }
    };
    div().pb_1().child(
        Button::new(SharedString::from(id))
            .ghost()
            .w_full()
            .h_10()
            .selected(selected == Some(&selection))
            .accessibility_label(format!("{title}. {subtitle}"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .w_full()
                    .min_w_0()
                    .child(Icon::new(icon).size_4().flex_none().text_color(color))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_sm().text_ellipsis().child(title))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .text_ellipsis()
                                    .child(subtitle),
                            ),
                    ),
            )
            .on_click(move |_, _, cx| {
                let _ = workspace.update(cx, |this, cx| {
                    if let Some(node) = this.node_workspaces.get_mut(&key)
                        && node.history_selection.as_ref() != Some(&selection)
                    {
                        node.history_selection = Some(selection.clone());
                        node.history_detail_scroll.set_offset(Default::default());
                        cx.notify();
                    }
                });
            }),
    )
}

fn run_row(status: &RunStatus, key: NodeKey, workspace: WeakEntity<Workspace>, cx: &App) -> Div {
    let id = status.run.id;
    let (state_icon, state_color) = run_state_visual(&status.run.state, cx.theme());
    let (snapshot, snapshot_icon, snapshot_color) = match &status.run.snapshot {
        SnapshotState::Saved { revision } => (
            format!("Snapshot saved · {}", short_id(&revision.commit_id)),
            IconName::CircleCheck,
            cx.theme().success,
        ),
        SnapshotState::Failed { detail } => (
            format!("Snapshot failed. {detail}"),
            IconName::CircleAlert,
            cx.theme().danger,
        ),
        _ => (
            "No snapshot recorded".to_owned(),
            IconName::CircleDashed,
            cx.theme().muted_foreground,
        ),
    };
    div()
        .border_b_1()
        .border_color(cx.theme().border)
        .pb_2()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .flex_wrap()
                .gap_1()
                .child(message(format!("Run #{id}")).font_semibold())
                .when(
                    matches!(status.run.snapshot, SnapshotState::Saved { .. }),
                    |row| {
                        row.child(
                            Button::new(SharedString::from(format!("run-{id}")))
                                .ghost()
                                .small()
                                .icon(IconName::GitCommitHorizontal)
                                .label("Review")
                                .accessibility_label("Review saved snapshot")
                                .tooltip("Review saved snapshot")
                                .on_click(move |_, _, cx| {
                                    let _ = workspace.update(cx, |this, cx| {
                                        this.open_review(key, Target::Run(id), cx)
                                    });
                                }),
                        )
                    },
                ),
        )
        .child(
            div()
                .flex()
                .items_start()
                .gap_1()
                .pt_1()
                .child(
                    Icon::new(state_icon)
                        .size_3()
                        .text_color(state_color)
                        .flex_none(),
                )
                .child(message(node_runs::state_label(&status.run.state))),
        )
        .child(
            div()
                .flex()
                .items_start()
                .gap_1()
                .pt_1()
                .child(
                    Icon::new(snapshot_icon)
                        .size_3()
                        .text_color(snapshot_color)
                        .flex_none(),
                )
                .child(message(snapshot).text_color(cx.theme().muted_foreground)),
        )
        .when_some(status.run.continued_from, |detail, previous| {
            detail.child(
                message(format!("Continued from run #{previous}"))
                    .text_color(cx.theme().muted_foreground),
            )
        })
}

fn revision_row(
    entry: &NodeRevisionEntry,
    key: NodeKey,
    workspace: WeakEntity<Workspace>,
    cx: &App,
) -> Div {
    let revision = &entry.evolution.revision;
    let commit = revision.commit_id.clone();
    let copy_commit = commit.clone();
    let short_commit = short_id(&commit);
    let operation = entry
        .evolution
        .operation_description
        .split_whitespace()
        .map(|word| {
            if word.len() == 40 && word.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                &word[..12]
            } else {
                word
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let description = revision
        .description
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| describe_operation(&operation));
    let mut metadata = div()
        .flex()
        .items_center()
        .flex_wrap()
        .gap_2()
        .text_color(cx.theme().muted_foreground)
        .child(message(readable_timestamp(&entry.evolution.timestamp)));
    if !operation.is_empty() && description != operation {
        metadata = metadata.child(message(operation));
    }
    if entry.is_current {
        metadata = metadata.child(message("Current head").text_color(cx.theme().link));
    }
    if revision.conflicted {
        metadata = metadata.child(message("Conflicts").text_color(cx.theme().warning));
    }
    if !entry.run_ids.is_empty() {
        metadata = metadata.child(message(
            entry
                .run_ids
                .iter()
                .map(|id| format!("Run #{id}"))
                .collect::<Vec<_>>()
                .join(" · "),
        ));
    }
    div()
        .border_b_1()
        .border_color(cx.theme().border)
        .py_2()
        .child(
            message(if description.is_empty() {
                "Native revision".to_owned()
            } else {
                description
            })
            .font_semibold(),
        )
        .child(metadata)
        .child(
            div()
                .flex()
                .items_center()
                .flex_wrap()
                .gap_2()
                .pt_1()
                .child(
                    Button::new(SharedString::from(format!("copy-{commit}")))
                        .ghost()
                        .small()
                        .icon(IconName::Copy)
                        .child(
                            div()
                                .font_family(cx.theme().mono_font_family.clone())
                                .child(short_commit),
                        )
                        .accessibility_label("Copy revision ID")
                        .tooltip(format!("Copy revision {commit}"))
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copy_commit.clone()))
                        }),
                )
                .child(
                    Button::new(SharedString::from(commit.clone()))
                        .ghost()
                        .small()
                        .icon(IconName::FileDiff)
                        .label("Review revision")
                        .on_click(move |_, _, cx| {
                            let _ = workspace.update(cx, |this, cx| {
                                this.open_review(key, Target::Revision(commit.clone()), cx)
                            });
                        }),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{ListAlignment, px};
    use serde_json::json;

    fn page(operation: &str, commits: &[&str], cursor: Option<&str>) -> NodeRevisionPage {
        serde_json::from_value(json!({
            "workflow_id": 7,
            "node_id": 1,
            "change_id": "change",
            "native_operation": operation,
            "current_commit_ids": ["head"],
            "entries": commits.iter().map(|commit| json!({
                "evolution": {
                    "revision": {"change_id": "change", "commit_id": commit, "description": "", "conflicted": false},
                    "predecessor_commit_ids": [],
                    "operation_id": operation,
                    "operation_description": "snapshot working copy",
                    "timestamp": "2026-11-04T12:00:00Z"
                },
                "is_current": *commit == "head",
                "run_ids": []
            })).collect::<Vec<_>>(),
            "next_cursor": cursor
        })).expect("page fixture matches the backend schema")
    }

    #[test]
    fn timestamps_reduce_to_a_consistent_minute_precision() {
        assert_eq!(
            readable_timestamp("2026-11-04T12:34:56Z"),
            "2026-11-04 12:34 UTC"
        );
        assert_eq!(
            readable_timestamp("2026-11-04T12:34:56.123+02:00"),
            "2026-11-04 12:34 +02:00"
        );
        assert_eq!(
            readable_timestamp("2026-11-04T20:06:54.452-08:00"),
            "2026-11-04 20:06 -08:00"
        );
        assert_eq!(readable_timestamp(" 2026-11-04 "), "2026-11-04");
        assert_eq!(readable_timestamp(""), "");
    }

    #[test]
    fn operation_descriptions_are_sentence_cased_without_inventing_text() {
        assert_eq!(
            describe_operation("snapshot working copy"),
            "Snapshot working copy"
        );
        assert_eq!(describe_operation("Already explicit"), "Already explicit");
        assert_eq!(describe_operation("éclair import"), "Éclair import");
        assert_eq!(describe_operation("  "), "");
    }

    #[test]
    fn older_page_keeps_existing_rows_and_anchor_until_pinned_page_succeeds() {
        let mut current = page("operation", &["head", "parent"], Some("opaque cursor"));
        let scroll = ListState::new(5, ListAlignment::Top, px(400.));
        scroll.scroll_to(gpui_kit::ListOffset {
            item_ix: 3,
            offset_in_item: px(6.),
        });
        let anchor = scroll.logical_scroll_top();

        assert!(
            append_older_page(
                &mut current,
                page("different operation", &["older"], None),
                &scroll
            )
            .is_err()
        );
        assert_eq!(current.entries.len(), 2);
        assert_eq!(current.next_cursor.as_deref(), Some("opaque cursor"));
        assert_eq!(scroll.item_count(), 5);

        append_older_page(&mut current, page("operation", &["older"], None), &scroll)
            .expect("same pinned history");
        assert_eq!(scroll.item_count(), 6);
        assert_eq!(scroll.logical_scroll_top(), anchor);
        assert_eq!(current.entries[0].evolution.revision.commit_id, "head");
        assert_eq!(current.entries[2].evolution.revision.commit_id, "older");
        assert!(current.next_cursor.is_none());
    }
    #[gpui_kit::test]
    fn retry_keeps_the_cursor_and_refresh_rejects_a_late_page(cx: &mut gpui_kit::TestAppContext) {
        use super::super::Tab;
        use crate::workspace::workspace_tests::{open_workspace, workflow};

        let (handle, workspace) = open_workspace(cx);
        let key = (1, 7, 1);
        cx.update_window(handle, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.graph = Load::Ready(
                    serde_json::from_value(json!({
                        "workflow": workflow("project-a", 7),
                        "nodes": [{"id": 1, "workflow_id": 7, "prompt": "History fixture"}],
                        "connections": []
                    }))
                    .expect("graph fixture matches the storage schema"),
                );
                workspace.open_node(1, window, cx);
                let node = workspace.node_workspaces.get_mut(&key).expect("open node");
                node.tab = Tab::History;
                let generation = node.begin_history_refresh().expect("initial load");
                let initial = page("before refresh", &["head"], Some("cursor"));
                assert!(
                    node.finish_history_refresh(generation, Load::Ready((vec![], Ok(initial))))
                );

                let (older, cursor) = node.begin_older_page().expect("older request");
                assert_eq!(cursor, "cursor");
                assert!(node.begin_older_page().is_none());
                assert!(node.finish_older_page(older, Err("Temporary error".into())));
                assert_eq!(node.older_error.as_deref(), Some("Temporary error"));
                assert_eq!(node.history_scroll.item_count(), 4);

                let (retry, cursor) = node.begin_older_page().expect("retry");
                assert_eq!(cursor, "cursor");
                assert!(node.older_error.is_none());
                let refreshed = node
                    .begin_history_refresh()
                    .expect("refresh while older page is pending");
                let current = page("after refresh", &["new head"], None);
                assert!(
                    node.finish_history_refresh(
                        refreshed,
                        Load::Ready((vec![], Ok(current.clone())))
                    )
                );
                assert!(
                    !node.finish_older_page(retry, Ok(page("before refresh", &["older"], None)))
                );
                let Load::Ready((_, Ok(result))) = &node.history else {
                    panic!("refreshed page stays ready");
                };
                assert_eq!(result, &current);
                assert_eq!(node.history_scroll.item_count(), 4);
            });
        })
        .expect("workspace window remains open");
    }

    #[gpui_kit::test]
    fn stale_history_is_announced_while_its_tab_stays_open(cx: &mut gpui_kit::TestAppContext) {
        use super::super::Tab;
        use crate::workspace::workspace_tests::{open_workspace, workflow};
        use gpui_kit::test::TestWindowExt as _;

        let (handle, workspace) = open_workspace(cx);
        let key = (1, 7, 1);
        cx.update_window(handle, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.graph = Load::Ready(
                    serde_json::from_value(json!({
                        "workflow": workflow("project-a", 7),
                        "nodes": [{"id": 1, "workflow_id": 7, "prompt": "History fixture"}],
                        "connections": []
                    }))
                    .expect("graph fixture matches the storage schema"),
                );
                workspace.open_node(1, window, cx);
                let node = workspace.node_workspaces.get_mut(&key).expect("open node");
                node.tab = Tab::History;
                node.history = Load::Ready((vec![], Ok(page("operation", &["head"], None))));
                node.history_stale = true;
                cx.notify();
            });
            window.render_frame(cx);
            assert!(window.find("history-stale").visible());
            assert!(window.find("reload-history").visible());
        })
        .expect("workspace window remains open");
    }
}
