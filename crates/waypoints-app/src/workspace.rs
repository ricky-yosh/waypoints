use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::base::{StyledExt as _, TestSupportExt as _};
use gpui_kit::component::{
    ActiveTheme as _, Icon, Selectable as _, Sizable as _, TitleBar,
    alert::Alert,
    button::{Button, ButtonVariants as _},
    menu::{DropdownMenu as _, PopupMenuItem},
    resizable::{HandleEdge, h_resizable, resizable_panel, resize_handle},
    spinner::Spinner,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Axis, Context, FocusHandle, InteractiveElement as _, IntoElement,
    KeyBinding, ParentElement as _, PathPromptOptions, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, actions, div, px, relative,
};
use waypoints_backend::{Client, Config};
use waypoints_core::storage::{Workflow, WorkflowGraph};

use crate::preferences::{
    Appearance, MAX_SIDEBAR_WIDTH, MIN_SIDEBAR_WIDTH, ProjectPreferences, SelectedWorkflow,
    WindowGeometry, WorkspacePreferences,
};

#[path = "node_workspace.rs"]
mod node_workspace;
#[path = "workflow_graph.rs"]
mod workflow_graph;

#[cfg(feature = "screenshot-fixtures")]
pub(crate) use node_workspace::ScreenshotSurface;

const DEFAULT_WORKBENCH_FRACTION: f32 = 0.74;
const MIN_WORKBENCH_FRACTION: f32 = 0.55;
const MAX_WORKBENCH_FRACTION: f32 = 0.92;

struct ResizeWorkbench;

impl Render for ResizeWorkbench {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

// Reserve a readable detail column when the navigator is expanded.
const MIN_DETAIL_WIDTH: f32 = 320.0;
// The divider hit area extends beyond the panel edge.
const DIVIDER_HIT_MARGIN: f32 = 5.0;

actions!(
    waypoints,
    [
        ToggleSidebar,
        AddProject,
        RefreshProject,
        FocusNext,
        FocusPrevious,
        CloseWorkbench,
        InspectGraphNode,
        DismissGraphPreview
    ]
);

pub(crate) fn init(cx: &mut App) {
    let (toggle, add, refresh) = if cfg!(target_os = "macos") {
        ("cmd-b", "cmd-o", "cmd-r")
    } else {
        ("ctrl-b", "ctrl-o", "ctrl-r")
    };
    cx.bind_keys([
        KeyBinding::new(toggle, ToggleSidebar, None),
        KeyBinding::new(add, AddProject, None),
        KeyBinding::new(refresh, RefreshProject, None),
        KeyBinding::new("tab", FocusNext, Some("WaypointsWorkspace")),
        KeyBinding::new("shift-tab", FocusPrevious, Some("WaypointsWorkspace")),
        KeyBinding::new("escape", CloseWorkbench, Some("WaypointsWorkbench")),
        KeyBinding::new("shift-enter", InspectGraphNode, Some("WaypointsGraphNode")),
        KeyBinding::new("escape", DismissGraphPreview, Some("WaypointsGraphPreview")),
    ]);
}

enum Load<T, E = String> {
    Idle,
    Loading,
    Ready(T),
    Error(E),
}

pub(crate) struct Workspace {
    pub(crate) appearance: Appearance,
    state: WorkspaceState,
    graph: Load<WorkflowGraph>,
    graph_revision: u64,
    workbench_fraction: f32,
    expanded_workbench: bool,
    open_workbenches: BTreeSet<(u64, i64)>,
    workbench_focus: FocusHandle,
    graph_cameras: BTreeMap<(u64, i64), workflow_graph::GraphCamera>,
    graph_preview: Option<i64>,
    graph_preview_keyboard: bool,
    graph_preview_task: Option<gpui_kit::Task<()>>,
    graph_preview_focus: FocusHandle,
    _preview_focus_out: gpui_kit::Subscription,
    graph_node_focus: BTreeMap<(u64, i64, i64), FocusHandle>,
    selected_nodes: BTreeMap<(u64, i64), i64>,
    node_workspaces: BTreeMap<(u64, i64, i64), node_workspace::NodeWorkspace>,
    sidebar_visible: bool,
    sidebar_width: f32,
    focus: FocusHandle,
    path_error: Option<String>,
    preferences_error: Option<String>,
    #[cfg(any(test, feature = "screenshot-fixtures"))]
    fixture_mode: bool,
}

struct Project {
    id: u64,
    path: PathBuf,
    expanded: bool,
    revision: u64,
    workflows: Load<Vec<Workflow>, ProjectLoadError>,
}

enum ProjectLoadError {
    DirectoryUnavailable(PathBuf),
    Backend(String),
}

#[derive(Default)]
struct WorkspaceState {
    projects: Vec<Project>,
    next_id: u64,
    selected: Option<(u64, i64)>,
}

impl WorkspaceState {
    fn add(&mut self, path: PathBuf) -> Option<u64> {
        if self.projects.iter().any(|project| project.path == path) {
            return None;
        }
        self.next_id += 1;
        let id = self.next_id;
        self.projects.push(Project {
            id,
            path,
            expanded: true,
            revision: 0,
            workflows: Load::Idle,
        });
        Some(id)
    }

    fn begin_load(&mut self, id: u64) -> Option<(PathBuf, u64)> {
        let project = self.projects.iter_mut().find(|project| project.id == id)?;
        if matches!(project.workflows, Load::Loading) {
            return None;
        }
        project.revision += 1;
        project.workflows = Load::Loading;
        Some((project.path.clone(), project.revision))
    }

    fn finish_load(
        &mut self,
        id: u64,
        revision: u64,
        result: Result<Vec<Workflow>, ProjectLoadError>,
    ) -> bool {
        let Some(project) = self
            .projects
            .iter_mut()
            .find(|project| project.id == id && project.revision == revision)
        else {
            return false;
        };
        if let (Some((selected_project, selected_workflow)), Ok(workflows)) =
            (self.selected, &result)
            && selected_project == id
            && !workflows
                .iter()
                .any(|workflow| workflow.id == selected_workflow)
        {
            self.selected = None;
        }
        project.workflows = match result {
            Ok(workflows) => Load::Ready(workflows),
            Err(error) => Load::Error(error),
        };
        true
    }

    fn remove(&mut self, id: u64) {
        self.projects.retain(|project| project.id != id);
        if self.selected.is_some_and(|(project, _)| project == id) {
            self.selected = None;
        }
    }
}

impl Workspace {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let graph_preview_focus = cx.focus_handle();
        let preview_focus_out = cx.on_focus_out(&graph_preview_focus, window, |this, _, _, cx| {
            if this.graph_preview_keyboard {
                this.graph_preview = None;
                this.graph_preview_keyboard = false;
                cx.notify();
            }
        });
        Self {
            appearance: Appearance::System,
            state: WorkspaceState::default(),
            graph: Load::Idle,
            graph_revision: 0,
            workbench_fraction: DEFAULT_WORKBENCH_FRACTION,
            expanded_workbench: false,
            open_workbenches: BTreeSet::new(),
            workbench_focus: cx.focus_handle(),
            graph_cameras: BTreeMap::new(),
            graph_preview: None,
            graph_preview_keyboard: false,
            graph_preview_task: None,
            graph_preview_focus,
            _preview_focus_out: preview_focus_out,
            graph_node_focus: BTreeMap::new(),
            selected_nodes: BTreeMap::new(),
            node_workspaces: BTreeMap::new(),
            sidebar_visible: true,
            sidebar_width: 240.0,
            focus,
            path_error: None,
            preferences_error: None,
            #[cfg(any(test, feature = "screenshot-fixtures"))]
            fixture_mode: cfg!(test),
        }
    }

    fn close_workbench(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(key) = self.state.selected {
            self.open_workbenches.remove(&key);
            self.expanded_workbench = false;
            let focus = self
                .selected_nodes
                .get(&key)
                .and_then(|id| self.graph_node_focus.get(&(key.0, key.1, *id)))
                .unwrap_or(&self.focus);
            focus.focus(window, cx);
            cx.notify();
        }
    }

    fn render_workbench(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        div()
            .id("workbench")
            .test_support()
            .key_context("WaypointsWorkbench")
            .track_focus(&self.workbench_focus)
            .on_action(cx.listener(|this, _: &CloseWorkbench, window, cx| {
                this.close_workbench(window, cx);
            }))
            .absolute()
            .occlude()
            .min_h_0()
            .min_w_0()
            .bg(cx.theme().popover.blend(cx.theme().muted.opacity(0.42)))
            .border_1()
            .border_color(cx.theme().ring.opacity(0.62))
            .when(self.expanded_workbench, |frame| frame.inset_0())
            .when(!self.expanded_workbench, |frame| {
                frame
                    .left_2()
                    .right_2()
                    .bottom_2()
                    .h(relative(self.workbench_fraction))
                    .rounded_lg()
                    .shadow_lg()
            })
            .child(self.render_node_workspace(window, cx))
            .when(!self.expanded_workbench, |frame| {
                frame.child(
                    resize_handle("workbench-resize", Axis::Vertical)
                        .inside(HandleEdge::Leading)
                        .with_appearance(Rc::new(|_, _, cx| {
                            Some(
                                div()
                                    .w_full()
                                    .flex()
                                    .justify_center()
                                    .child(
                                        div()
                                            .w_8()
                                            .h_0p5()
                                            .rounded_full()
                                            .bg(cx.theme().muted_foreground),
                                    )
                                    .into_any_element(),
                            )
                        }))
                        .on_drag(ResizeWorkbench, |_, _, _, cx| cx.new(|_| ResizeWorkbench)),
                )
            })
    }

    fn open_project(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Add project".into()),
        });
        cx.spawn(async move |this, cx| {
            let result = receiver.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.into_iter().next() {
                            this.add_project(path, cx);
                        }
                    }
                    Ok(Ok(None)) => {}
                    Ok(Err(error)) => {
                        this.path_error = Some(format!("Couldn’t open folder picker: {error}"))
                    }
                    Err(error) => {
                        this.path_error = Some(format!("Couldn’t open folder picker: {error}"))
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn add_project(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let path = cx
                .background_spawn(async move { normalize_project_path(path) })
                .await;
            this.update(cx, |this, cx| {
                match this.state.add(path) {
                    Some(id) => {
                        this.path_error = None;
                        this.refresh(id, cx);
                    }
                    None => this.path_error = Some("This project is already open.".into()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn toggle_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_visible = !self.sidebar_visible;
        if !self.sidebar_visible {
            self.focus.focus(window, cx);
        }
        cx.notify();
    }

    fn refresh(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some((project, revision)) = self.state.begin_load(id) else {
            return;
        };
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if !project.is_dir() {
                        return Err(ProjectLoadError::DirectoryUnavailable(project));
                    }
                    run_backend(|client: Client| async move {
                        client
                            .list_workflows(&project)
                            .await
                            .map_err(|e| e.to_string())
                    })
                    .map_err(ProjectLoadError::Backend)
                })
                .await;
            this.update(cx, |this, cx| {
                let selected = this.state.selected;
                if !this.state.finish_load(id, revision, result) {
                    return;
                }
                if selected != this.state.selected {
                    this.graph_revision += 1;
                    this.graph = Load::Idle;
                } else if let Some((project, workflow)) =
                    selected.filter(|(project, _)| *project == id)
                {
                    this.load_graph(project, workflow, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn select(&mut self, project: u64, id: i64, cx: &mut Context<Self>) {
        if !self.state.projects.iter().any(|entry| entry.id == project && matches!(&entry.workflows, Load::Ready(workflows) if workflows.iter().any(|workflow| workflow.id == id))) {
            return;
        }
        if self.state.selected == Some((project, id)) && matches!(self.graph, Load::Loading) {
            return;
        }
        self.load_graph(project, id, cx);
    }

    fn load_graph(&mut self, project: u64, id: i64, cx: &mut Context<Self>) {
        if self.state.selected != Some((project, id)) {
            self.expanded_workbench = false;
        }
        self.graph_preview = None;
        self.graph_preview_keyboard = false;
        self.graph_preview_task = None;
        self.state.selected = Some((project, id));
        self.graph_revision += 1;
        let revision = self.graph_revision;
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
                if this.graph_revision != revision || this.state.selected != Some((project, id)) {
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

impl Workspace {
    pub(crate) fn restore_preferences(
        &mut self,
        preferences: WorkspacePreferences,
        cx: &mut Context<Self>,
    ) {
        self.appearance = preferences.appearance;
        self.sidebar_visible = preferences.sidebar_visible;
        self.sidebar_width = preferences.sidebar_width;
        self.workbench_fraction = preferences
            .workbench_fraction
            .filter(|fraction| (MIN_WORKBENCH_FRACTION..=MAX_WORKBENCH_FRACTION).contains(fraction))
            .unwrap_or(DEFAULT_WORKBENCH_FRACTION);
        for saved in preferences.projects {
            if let Some(id) = self.state.add(saved.path.clone())
                && let Some(project) = self.state.projects.iter_mut().find(|entry| entry.id == id)
            {
                project.expanded = saved.expanded;
                if let Some(selected) = &preferences.selected
                    && selected.project == saved.path
                {
                    self.state.selected = Some((id, selected.workflow_id));
                }
            }
        }
        let ids: Vec<_> = self
            .state
            .projects
            .iter()
            .map(|project| project.id)
            .collect();
        for id in ids {
            self.refresh(id, cx);
        }
        cx.notify();
    }

    pub(crate) fn snapshot_preferences(&self, window: &Window) -> WorkspacePreferences {
        let bounds = window.window_bounds().get_bounds();
        WorkspacePreferences {
            appearance: self.appearance,
            projects: self
                .state
                .projects
                .iter()
                .map(|project| ProjectPreferences {
                    path: project.path.clone(),
                    expanded: project.expanded,
                })
                .collect(),
            selected: self.state.selected.and_then(|(project, workflow_id)| {
                self.state
                    .projects
                    .iter()
                    .find(|entry| entry.id == project)
                    .map(|entry| SelectedWorkflow {
                        project: entry.path.clone(),
                        workflow_id,
                    })
            }),
            sidebar_width: self.sidebar_width,
            sidebar_visible: self.sidebar_visible,
            workbench_fraction: Some(self.workbench_fraction),
            window_bounds: Some(WindowGeometry {
                x: bounds.origin.x.into(),
                y: bounds.origin.y.into(),
                width: bounds.size.width.into(),
                height: bounds.size.height.into(),
            }),
        }
    }

    pub(crate) fn show_preferences_error(&mut self, error: String, cx: &mut Context<Self>) {
        self.preferences_error = Some(error);
        cx.notify();
    }
}

// Folder identity requires filesystem access; callers run this off the UI thread.
fn normalize_project_path(path: PathBuf) -> PathBuf {
    std::fs::canonicalize(&path)
        .or_else(|_| std::path::absolute(&path))
        .unwrap_or(path)
}

pub(crate) fn normalize_preferences(mut preferences: WorkspacePreferences) -> WorkspacePreferences {
    for project in &mut preferences.projects {
        project.path = normalize_project_path(std::mem::take(&mut project.path));
    }
    if let Some(selected) = &mut preferences.selected {
        selected.project = normalize_project_path(std::mem::take(&mut selected.project));
    }
    preferences
}

#[cfg(test)]
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
        let config = Config::default_paths().map_err(|e| e.to_string())?;
        let client = Client::connect(&config).await.map_err(|e| format!(
            "Couldn’t connect to the backend: {e}. Start it with `cargo run -p waypoints-cli -- workflow list --project .`, then refresh."
        ))?;
        f(client).await
    })
}

fn message(text: impl Into<SharedString>) -> gpui_kit::Div {
    div().text_sm().child(text.into())
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A project refresh can replace a saved assignment while its draft
        // workspace is cached. Reconcile the selected node before rendering so
        // the editor never presents the previous saved text as current.
        self.sync_selected_assignment(window, cx);
        let mut list = div().flex().flex_col().gap_1();
        if self.state.projects.is_empty() {
            list = list.child(
                message("No projects open")
                    .px_2()
                    .text_color(cx.theme().muted_foreground),
            );
        }
        for project in &self.state.projects {
            let project_id = project.id;
            let path = project.path.to_string_lossy();
            let name = project.path.file_name().map_or_else(
                || path.to_string(),
                |name| name.to_string_lossy().into_owned(),
            );
            let duplicate_name = self.state.projects.iter().any(|other| {
                other.id != project_id && other.path.file_name() == project.path.file_name()
            });
            let name = if duplicate_name {
                project.path.parent().map_or(name.clone(), |parent| {
                    let parent_name = parent.file_name();
                    let duplicate_parent = self.state.projects.iter().any(|other| {
                        other.id != project_id
                            && other.path.file_name() == project.path.file_name()
                            && other.path.parent().and_then(std::path::Path::file_name)
                                == parent_name
                    });
                    let qualifier = parent_name
                        .filter(|_| !duplicate_parent)
                        .map_or_else(|| parent.to_string_lossy(), |name| name.to_string_lossy());
                    format!("{name} · {qualifier}")
                })
            } else {
                name
            };
            let weak = cx.entity().downgrade();
            let loading = matches!(project.workflows, Load::Loading);
            let refresh_label = if matches!(project.workflows, Load::Error(_)) {
                "Retry"
            } else {
                "Refresh"
            };
            let header = div()
                .flex()
                .items_center()
                .child(
                    Button::new(("project", project_id))
                        .small()
                        .ghost()
                        .flex_1()
                        .min_w_0()
                        .accessibility_label(format!(
                            "{} {name}",
                            if project.expanded {
                                "Collapse"
                            } else {
                                "Expand"
                            }
                        ))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .w_full()
                                .min_w_0()
                                .child(
                                    Icon::new(if project.expanded {
                                        IconName::ChevronDown
                                    } else {
                                        IconName::ChevronRight
                                    })
                                    .size_3()
                                    .flex_shrink_0(),
                                )
                                .child(div().min_w_0().text_ellipsis().child(name)),
                        )
                        .tooltip(path.to_string())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(project) =
                                this.state.projects.iter_mut().find(|p| p.id == project_id)
                            {
                                project.expanded = !project.expanded;
                            }
                            cx.notify();
                        })),
                )
                .child(
                    Button::new(("project-menu", project_id))
                        .small()
                        .ghost()
                        .icon(IconName::Ellipsis)
                        .accessibility_label(format!("Project actions for {path}"))
                        .tooltip("Project actions")
                        .dropdown_menu(move |menu, _, _| {
                            let refresh = weak.clone();
                            let remove = weak.clone();
                            menu.item(
                                PopupMenuItem::new(refresh_label)
                                    .disabled(loading)
                                    .on_click(move |_, _, cx| {
                                        let _ = refresh.update(cx, |this, cx| {
                                            this.refresh(project_id, cx);
                                        });
                                    }),
                            )
                            .item(
                                PopupMenuItem::new("Remove from Sidebar").on_click(
                                    move |_, window, cx| {
                                        let _ = remove.update(cx, |this, cx| {
                                            this.focus.focus(window, cx);
                                            let selected = this.state.selected;
                                            this.state.remove(project_id);
                                            if selected != this.state.selected {
                                                this.graph_revision += 1;
                                                this.graph = Load::Idle;
                                            }
                                            cx.notify();
                                        });
                                    },
                                ),
                            )
                        }),
                );
            let mut entry = div().flex().flex_col().gap_1().child(header);
            if project.expanded {
                let mut contents = div().flex().flex_col().gap_1().pl_4();
                contents = match &project.workflows {
                    Load::Idle | Load::Loading => contents.child(
                        div()
                            .id(("project-loading", project_id))
                            .role(Role::Status)
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .py_1()
                            .text_color(cx.theme().muted_foreground)
                            .child(Spinner::new().xsmall())
                            .child(message("Loading workflows…")),
                    ),
                    Load::Error(error) => {
                        let (status, diagnostic) = match error {
                            ProjectLoadError::DirectoryUnavailable(path) => (
                                "Folder unavailable",
                                format!("The project folder is unavailable at {}", path.display()),
                            ),
                            ProjectLoadError::Backend(error) => {
                                ("Couldn’t load workflows", error.clone())
                            }
                        };
                        contents.child(
                            div()
                                .id(("project-error", project_id))
                                .role(Role::Status)
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap_x_1()
                                .pl_2()
                                .child(message(status).text_color(cx.theme().muted_foreground))
                                .child(
                                    Button::new(("retry-project", project_id))
                                        .xsmall()
                                        .ghost()
                                        .label("Retry")
                                        .tooltip(diagnostic)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.refresh(project_id, cx);
                                        })),
                                ),
                        )
                    }
                    Load::Ready(workflows) if workflows.is_empty() => contents.child(
                        message("No active workflows")
                            .px_2()
                            .py_1()
                            .text_color(cx.theme().muted_foreground),
                    ),
                    Load::Ready(workflows) => {
                        for workflow in workflows {
                            let id = workflow.id;
                            contents = contents.child(
                                Button::new(SharedString::from(format!(
                                    "workflow-{project_id}-{id}"
                                )))
                                .small()
                                .accessibility_label(workflow.name.clone())
                                .child(
                                    div()
                                        .w_full()
                                        .min_w_0()
                                        .text_ellipsis()
                                        .child(workflow.name.clone()),
                                )
                                .tooltip(workflow.name.clone())
                                .ghost()
                                .selected(self.state.selected == Some((project_id, id)))
                                .on_click(cx.listener(
                                    move |this, _, _, cx| this.select(project_id, id, cx),
                                )),
                            );
                        }
                        contents
                    }
                };
                entry = entry.child(contents);
            }
            list = list.child(entry);
        }
        let mut detail = div().flex().flex_col().gap_3();
        let selected_project = self
            .state
            .selected
            .and_then(|(id, _)| self.state.projects.iter().find(|project| project.id == id));
        if let Some(project) = selected_project {
            detail = detail.child(
                message(project.path.display().to_string()).text_color(cx.theme().muted_foreground),
            );
        }
        match &self.graph {
            Load::Idle => {
                let prompt = match selected_project.map(|project| &project.workflows) {
                    Some(Load::Error(_)) => "Retry the project to reopen the selected workflow.",
                    Some(_) => "Opening the selected workflow…",
                    None if self.state.projects.is_empty() => "Add a project to see its workflows.",
                    None => "Select a workflow to see its nodes and dependencies.",
                };
                detail = detail.child(
                    message(prompt)
                        .id("detail-status")
                        .role(Role::Status)
                        .text_color(cx.theme().muted_foreground),
                );
            }
            Load::Loading => {
                detail = detail.child(
                    message("Loading workflow…")
                        .id("graph-loading")
                        .role(Role::Status),
                )
            }
            Load::Error(error) => {
                detail = detail.child(
                    Alert::error("graph-error", format!("Couldn’t load workflow: {error}."))
                        .small(),
                );
                if let Some((project, id)) = self.state.selected {
                    detail =
                        detail.child(Button::new("retry-graph").label("Retry workflow").on_click(
                            cx.listener(move |this, _, _, cx| this.load_graph(project, id, cx)),
                        ));
                }
            }
            Load::Ready(_) => {}
        }
        let sidebar = div()
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .gap_2()
            .p_2()
            .bg(cx.theme().sidebar)
            .text_color(cx.theme().sidebar_foreground)
            .child(div().px_2().text_sm().child("Projects"))
            .child(
                div()
                    .id("workflows")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(list),
            );
        let detail = if let (Some(key), Load::Ready(graph)) = (self.state.selected, &self.graph) {
            let selected = self.selected_nodes.get(&key).copied();
            self.graph_cameras.entry(key).or_default();
            for node in &graph.nodes {
                self.graph_node_focus
                    .entry((key.0, key.1, node.id))
                    .or_insert_with(|| cx.focus_handle());
            }
            let workbench_open = self.open_workbenches.contains(&key)
                && selected.is_some_and(|id| graph.nodes.iter().any(|node| node.id == id));
            let canvas = div()
                .id("graph-canvas")
                .relative()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .on_drag_move(cx.listener(
                    |this, event: &gpui_kit::DragMoveEvent<ResizeWorkbench>, window, cx| {
                        let height = event.bounds.size.height;
                        if height > px(0.) {
                            let margin = window.rem_size() * 0.5;
                            this.workbench_fraction =
                                ((event.bounds.bottom() - event.event.position.y - margin)
                                    / height)
                                    .clamp(MIN_WORKBENCH_FRACTION, MAX_WORKBENCH_FRACTION);
                            cx.notify();
                        }
                    },
                ))
                .child(self.render_graph(graph, key, selected, window, cx))
                .when(workbench_open, |canvas| {
                    canvas.child(self.render_workbench(window, cx))
                });
            div().id("detail").size_full().min_w_0().min_h_0().flex().flex_col()
                .child(div().flex().items_center().justify_between().gap_2().px_3().py_2()
                    .border_b_1().border_color(cx.theme().border)
                    .child(div().min_w_0().font_semibold().overflow_hidden().text_ellipsis()
                        .child(graph.workflow.name.clone()))
                    .child(self.permission_actions(cx))
                    .child(self.graph_zoom_controls(key, cx)))
                .when(graph.nodes.is_empty(), |detail| detail.child(message("No nodes in this workflow.").px_3()))
                .when(graph.unresolved_dependency_operation.is_some(), |detail| {
                    detail.child(Alert::warning("graph-unresolved", "Saved edges may differ from jj while a dependency operation is unresolved.").small())
                })
                .child(canvas)
        } else {
            div()
                .id("detail")
                .size_full()
                .min_w_0()
                .min_h_0()
                .overflow_y_scroll()
                .p_4()
                .child(detail)
        };
        let weak = cx.entity().downgrade();
        let max_sidebar_width =
            (f32::from(window.viewport_size().width) - MIN_DETAIL_WIDTH - DIVIDER_HIT_MARGIN)
                .clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH);
        let content = if self.sidebar_visible {
            h_resizable("workspace-split")
                .on_resize(move |state, _, cx| {
                    if let Some(width) = state.read(cx).sizes().first().copied() {
                        let width: f32 = width.into();
                        weak.update(cx, |this, cx| {
                            this.sidebar_width = width;
                            cx.notify();
                        })
                        .ok();
                    }
                })
                .child(
                    resizable_panel()
                        .size(px(self
                            .sidebar_width
                            .clamp(MIN_SIDEBAR_WIDTH, max_sidebar_width)))
                        .size_range(px(MIN_SIDEBAR_WIDTH)..px(max_sidebar_width))
                        .fixed()
                        .child(sidebar),
                )
                .child(resizable_panel().child(detail))
                .into_any_element()
        } else {
            detail.into_any_element()
        };
        let toggle_label = if self.sidebar_visible {
            "Hide projects"
        } else {
            "Show projects"
        };
        let toolbar = div()
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .gap_1()
            .px_2()
            .child(
                Button::new("toggle-sidebar")
                    .small()
                    .ghost()
                    .icon(IconName::PanelLeft)
                    .accessibility_label(toggle_label)
                    .tooltip(toggle_label)
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_sidebar(window, cx))),
            )
            .child(
                Button::new("open")
                    .small()
                    .ghost()
                    .icon(IconName::Plus)
                    .accessibility_label("Add Project…")
                    .tooltip("Add Project…")
                    .on_click(cx.listener(|this, _, _, cx| this.open_project(cx))),
            )
            .child(div().ml_auto().px_2().text_sm().child("Waypoints"));
        let toolbar = if cfg!(target_os = "macos") {
            TitleBar::new().child(toolbar).into_any_element()
        } else {
            toolbar
                .py_1()
                .flex_grow_0()
                .border_b_1()
                .border_color(cx.theme().sidebar_border)
                .bg(cx.theme().sidebar)
                .into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .track_focus(&self.focus)
            .key_context("WaypointsWorkspace")
            .tab_group()
            .on_action(|_: &FocusNext, window, cx| window.focus_next(cx))
            .on_action(|_: &FocusPrevious, window, cx| window.focus_prev(cx))
            .on_action(cx.listener(|this, _: &ToggleSidebar, window, cx| {
                this.toggle_sidebar(window, cx);
            }))
            .on_action(cx.listener(|this, _: &AddProject, _, cx| {
                this.open_project(cx);
            }))
            .on_action(cx.listener(|this, _: &RefreshProject, window, cx| {
                if let Some((project, _)) = this.state.selected {
                    this.focus.focus(window, cx);
                    this.refresh(project, cx);
                }
            }))
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(toolbar)
            .when_some(self.path_error.clone(), |this, error| {
                let workspace = cx.entity().downgrade();
                this.child(
                    Alert::error("project-picker-error", error)
                        .small()
                        .on_close(move |_, window, cx| {
                            let _ = workspace.update(cx, |workspace, cx| {
                                workspace.path_error = None;
                                workspace.focus.focus(window, cx);
                                cx.notify();
                            });
                        }),
                )
            })
            .when_some(self.preferences_error.clone(), |this, error| {
                this.child(
                    Alert::warning(
                        "preferences-error",
                        format!("Workspace preferences: {error}"),
                    )
                    .small(),
                )
            })
            .child(div().flex_1().min_h_0().min_w_0().child(content))
    }
}

#[cfg(test)]
#[path = "workspace_tests.rs"]
mod workspace_tests;

#[cfg(feature = "screenshot-fixtures")]
#[path = "workspace_fixtures.rs"]
pub(crate) mod fixtures;

#[cfg(feature = "native-qa")]
#[path = "workspace_qa.rs"]
pub(crate) mod qa;

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

    #[test]
    fn project_requests_are_independent_and_removed_instances_reject_results() {
        let mut state = WorkspaceState::default();
        let a = state.add("missing-a".into()).expect("first project");
        let b = state.add("missing-b".into()).expect("second project");
        let (_, a_revision) = state.begin_load(a).expect("load a");
        let (_, b_revision) = state.begin_load(b).expect("load b");
        assert!(state.finish_load(a, a_revision, Ok(vec![])));
        assert!(matches!(state.projects[1].workflows, Load::Loading));
        state.selected = Some((b, 42));
        state.remove(b);
        assert_eq!(state.selected, None);
        let replacement = state.add("missing-b".into()).expect("re-add project");
        assert_ne!(replacement, b);
        assert!(!state.finish_load(
            b,
            b_revision,
            Err(ProjectLoadError::Backend("stale".into()))
        ));
        assert!(matches!(state.projects[1].workflows, Load::Idle));
        let (_, old) = state.begin_load(replacement).expect("old request");
        assert!(
            state.begin_load(replacement).is_none(),
            "duplicate refresh ignored"
        );
        assert!(state.finish_load(replacement, old, Ok(vec![])));
        let (_, current) = state.begin_load(replacement).expect("new request");
        assert!(!state.finish_load(replacement, old, Ok(vec![])));
        assert!(state.finish_load(replacement, current, Ok(vec![])));
    }
}
