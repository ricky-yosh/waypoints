use std::path::PathBuf;

use gpui_kit::base::StyledExt as _;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState},
};
use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
};
use waypoints_backend::{Client, Config};
use waypoints_core::storage::{Workflow, WorkflowGraph};

enum Load<T> {
    Idle,
    Loading,
    Ready(T),
    Error(String),
}

pub(crate) struct Workspace {
    path: Entity<InputState>,
    project: Option<PathBuf>,
    workflows: Load<Vec<Workflow>>,
    selected: Option<i64>,
    graph: Load<WorkflowGraph>,
    revision: u64,
    _subscription: Subscription,
}

impl Workspace {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let path = cx.new(|cx| InputState::new(window, cx).placeholder("Project directory"));
        let subscription = cx.subscribe_in(&path, window, |this, _, event, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.open_project(cx);
            }
        });
        Self {
            path,
            project: None,
            workflows: Load::Idle,
            selected: None,
            graph: Load::Idle,
            revision: 0,
            _subscription: subscription,
        }
    }

    fn open_project(&mut self, cx: &mut Context<Self>) {
        if matches!(self.workflows, Load::Loading) {
            return;
        }
        let path = self.path.read(cx).value().to_string();
        if path.trim().is_empty() {
            self.revision += 1;
            self.project = None;
            self.selected = None;
            self.graph = Load::Idle;
            self.workflows = Load::Error("Enter a project directory.".into());
            cx.notify();
            return;
        }
        self.project = Some(PathBuf::from(path));
        self.selected = None;
        self.graph = Load::Idle;
        self.refresh(cx);
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        if matches!(self.workflows, Load::Loading) {
            return;
        }
        let Some(project) = self.project.clone() else {
            return;
        };
        self.revision += 1;
        let revision = self.revision;
        let selected = self.selected;
        self.workflows = Load::Loading;
        self.graph = if selected.is_some() {
            Load::Loading
        } else {
            Load::Idle
        };
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    run_backend(|client: Client| async move {
                        load_project(&client, &project, selected).await
                    })
                })
                .await;
            this.update(cx, |this, cx| {
                if this.revision != revision {
                    return;
                }
                match result {
                    Ok((workflows, graph)) => {
                        if graph.is_none() {
                            this.selected = None;
                        }
                        this.graph = match graph {
                            Some(Ok(graph)) => Load::Ready(graph),
                            Some(Err(error)) => Load::Error(error),
                            None => Load::Idle,
                        };
                        this.workflows = Load::Ready(workflows);
                    }
                    Err(error) => {
                        this.workflows = Load::Error(error);
                        this.graph = Load::Idle;
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn select(&mut self, id: i64, cx: &mut Context<Self>) {
        if !can_select(&self.workflows, self.selected, &self.graph, id) {
            return;
        }
        self.selected = Some(id);
        self.revision += 1;
        let revision = self.revision;
        self.graph = Load::Loading;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    run_backend(|client: Client| async move {
                        client.workflow_graph(id).await.map_err(|e| e.to_string())
                    })
                })
                .await;
            this.update(cx, |this, cx| {
                if this.revision != revision {
                    return;
                }
                this.graph = match result {
                    Ok(graph) => Load::Ready(graph),
                    Err(error) => Load::Error(error),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

fn can_select(
    workflows: &Load<Vec<Workflow>>,
    selected: Option<i64>,
    graph: &Load<WorkflowGraph>,
    id: i64,
) -> bool {
    matches!(workflows, Load::Ready(workflows) if workflows.iter().any(|workflow| workflow.id == id))
        && !(selected == Some(id) && matches!(graph, Load::Loading))
}

async fn load_project(
    client: &Client,
    project: &std::path::Path,
    selected: Option<i64>,
) -> Result<(Vec<Workflow>, Option<Result<WorkflowGraph, String>>), String> {
    let workflows = client
        .list_workflows(project)
        .await
        .map_err(|e| e.to_string())?;
    let graph = match selected.filter(|id| workflows.iter().any(|w| w.id == *id)) {
        Some(id) => Some(client.workflow_graph(id).await.map_err(|e| e.to_string())),
        None => None,
    };
    Ok((workflows, graph))
}

// Client uses Tokio sockets and timers. A short-lived runtime on a background
// worker keeps both connection setup and every request off the GPUI thread.
fn run_backend<T>(f: impl AsyncFnOnce(Client) -> Result<T, String>) -> Result<T, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    runtime.block_on(async {
        let database = state_directory()?.join("workflows.db");
        let config = Config::new(&database, &database.with_extension("sock")).map_err(|e| e.to_string())?;
        let client = Client::connect(&config).await.map_err(|e| format!(
            "Couldn’t connect to the backend: {e}. Start it with `cargo run -p waypoints-cli -- workflow list --project .`, then refresh."
        ))?;
        f(client).await
    })
}

fn state_directory() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("XDG_STATE_HOME").filter(|p| !p.is_empty()) {
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err("XDG_STATE_HOME must be absolute.".into());
        }
        return Ok(path.join("waypoints"));
    }
    let home = PathBuf::from(std::env::var_os("HOME").ok_or("HOME is not set.")?);
    #[cfg(target_os = "macos")]
    let path = home.join("Library/Application Support/waypoints");
    #[cfg(not(target_os = "macos"))]
    let path = home.join(".local/state/waypoints");
    Ok(path)
}

fn message(text: impl Into<SharedString>) -> gpui_kit::Div {
    div().text_sm().child(text.into())
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = div().flex().flex_col().gap_2();
        match &self.workflows {
            Load::Idle => list = list.child(message("Open a project to see its workflows.")),
            Load::Loading => list = list.child(message("Loading workflows…")),
            Load::Error(error) => {
                list = list.child(message(format!("Couldn’t load workflows: {error}")))
            }
            Load::Ready(workflows) if workflows.is_empty() => {
                list = list.child(message("No active workflows in this project."))
            }
            Load::Ready(workflows) => {
                for workflow in workflows {
                    let id = workflow.id;
                    list = list.child(
                        Button::new(("workflow", id as u64))
                            .label(format!("{}  (#{id})", workflow.name))
                            .ghost()
                            .selected(self.selected == Some(id))
                            .on_click(cx.listener(move |this, _, _, cx| this.select(id, cx))),
                    );
                }
            }
        }
        let mut detail = div().flex().flex_col().gap_3();
        match &self.graph {
            Load::Idle => {
                detail = detail.child(message(
                    "Select a workflow to see its nodes and dependencies.",
                ))
            }
            Load::Loading => detail = detail.child(message("Loading workflow…")),
            Load::Error(error) => {
                detail = detail.child(message(format!(
                    "Couldn’t load workflow: {error}. Refresh to try again."
                )))
            }
            Load::Ready(graph) => {
                detail = detail.child(div().text_lg().child(graph.workflow.name.clone()));
                detail = detail.child(message(if graph.workflow.base.is_some() {
                    "Saved graph · arrows point from upstream to downstream"
                } else {
                    "Unbased workflow · saved graph · arrows point from upstream to downstream"
                }));
                if graph.unresolved_dependency_operation.is_some() {
                    detail = detail.child(message(
                        "An unresolved dependency operation may mean saved edges differ from jj.",
                    ));
                }
                detail = detail.child(div().font_semibold().child("Nodes"));
                if graph.nodes.is_empty() {
                    detail = detail.child(message("No nodes in this workflow."));
                }
                for node in &graph.nodes {
                    let base = if graph.base_nodes.contains(&node.id) {
                        " · attached to base"
                    } else {
                        ""
                    };
                    detail = detail.child(message(format!("#{}{base}: {}", node.id, node.prompt)));
                }
                detail = detail.child(div().font_semibold().child("Dependencies"));
                if graph.connections.is_empty() {
                    detail = detail.child(message("No node dependencies."));
                }
                for edge in &graph.connections {
                    detail = detail.child(message(format!(
                        "#{} → #{}",
                        edge.source_id, edge.target_id
                    )));
                }
            }
        }
        div()
            .flex()
            .flex_col()
            .size_full()
            .p_4()
            .gap_3()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(div().text_lg().font_semibold().child("Waypoints"))
            .child(div().child("Project directory"))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(Input::new(&self.path)))
                    .child(
                        Button::new("open")
                            .label("Open")
                            .disabled(matches!(self.workflows, Load::Loading))
                            .on_click(cx.listener(|this, _, _, cx| this.open_project(cx))),
                    )
                    .child(
                        Button::new("refresh")
                            .label("Refresh")
                            .disabled(
                                self.project.is_none() || matches!(self.workflows, Load::Loading),
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .gap_4()
                    .items_stretch()
                    .child(
                        div()
                            .w_64()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(div().font_semibold().child("Workflows"))
                            .child(div().id("workflows").overflow_y_scroll().child(list)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .id("detail")
                            .overflow_y_scroll()
                            .child(detail),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn refresh_reads_external_changes_and_drops_removed_selection() {
        let directory = tempfile::Builder::new()
            .prefix("wp-")
            .tempdir_in("/tmp")
            .expect("temporary project");
        let config = Config::new(&directory.path().join("db"), &directory.path().join("sock"))
            .expect("test backend config");
        let server_config = config.clone();
        let server = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("server runtime")
                .block_on(waypoints_backend::serve(&server_config))
        });
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("client runtime");
        runtime.block_on(async {
            let client = tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    if let Ok(client) = Client::connect(&config).await {
                        break client;
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            })
            .await
            .expect("backend starts");
            let (empty, _) = load_project(&client, directory.path(), None)
                .await
                .expect("empty list");
            assert!(empty.is_empty());
            let workflow = client
                .create_workflow(directory.path(), "External")
                .await
                .expect("create");
            let (workflows, graph) = load_project(&client, directory.path(), Some(workflow.id))
                .await
                .expect("refresh");
            assert_eq!(workflows.len(), 1);
            assert_eq!(
                graph.expect("selected graph").expect("graph").workflow.id,
                workflow.id
            );
            assert!(!can_select(&Load::Loading, None, &Load::Idle, workflow.id));
            assert!(!can_select(
                &Load::Ready(workflows),
                Some(workflow.id),
                &Load::Loading,
                workflow.id
            ));
            client.archive_workflow(workflow.id).await.expect("archive");
            let (workflows, graph) = load_project(&client, directory.path(), Some(workflow.id))
                .await
                .expect("refresh after archive");
            assert!(workflows.is_empty());
            assert!(graph.is_none());
            client.stop().await.expect("stop server");
        });
        server
            .join()
            .expect("server thread")
            .expect("server shutdown");
    }
}
