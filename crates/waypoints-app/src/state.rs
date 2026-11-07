//! Backend-connected application state, requests, and polling.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use rune::{TextEdit, Waker};
use rune_kit::diff::{self, DiffState};
use waypoints_backend::RunStatus;
use waypoints_backend::activity::ActivityLog;
use waypoints_core::jj::RevisionReview;
use waypoints_core::run::{ContinuationAssessment, SnapshotState};
use waypoints_core::storage::{NodeRevisionEntry, Workflow, WorkflowGraph};

use crate::backend::{Backend, Command, ReviewTarget, Update};
use crate::canvas;
use crate::fixture::{self, Graph};
use crate::preferences::{self, Appearance, RunePreferences};
use crate::startup::startup;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Fixture,
    Backend,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LoadState {
    Idle,
    Loading,
    Ready,
    Failed(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum WorkbenchTab {
    #[default]
    Assignment,
    Output,
    Changes,
    History,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum HistoryKind {
    #[default]
    Runs,
    Revisions,
}

pub(crate) struct HistoryPanel {
    pub kind: HistoryKind,
    pub runs: Vec<RunStatus>,
    pub runs_cursor: Option<String>,
    pub runs_state: LoadState,
    pub revisions: Vec<NodeRevisionEntry>,
    pub revisions_cursor: Option<String>,
    pub revisions_state: LoadState,
    pub token: u64,
    pub selected_run: Option<i64>,
    pub selected_revision: Option<String>,
    pub notice: Option<String>,
    pub notice_error: bool,
    pub fresh: bool,
    pub list_width: f32,
    pub list_resize_start: Option<f32>,
}

impl Default for HistoryPanel {
    fn default() -> Self {
        Self {
            kind: HistoryKind::Runs,
            runs: Vec::new(),
            runs_cursor: None,
            runs_state: LoadState::Idle,
            revisions: Vec::new(),
            revisions_cursor: None,
            revisions_state: LoadState::Idle,
            token: 0,
            selected_run: None,
            selected_revision: None,
            notice: None,
            notice_error: false,
            fresh: false,
            list_width: 270.0,
            list_resize_start: None,
        }
    }
}

pub(crate) struct ReviewPanel {
    pub tab: WorkbenchTab,
    pub target: ReviewTarget,
    pub state: LoadState,
    pub token: u64,
    pub review: Option<RevisionReview>,
    pub diff: DiffState,
    pub stale: bool,
    pub loaded_commit: Option<String>,
    pub selected_file: Option<usize>,
    pub file_list_width: f32,
    pub file_resize_start: Option<f32>,
    pub history: HistoryPanel,
}

impl Default for ReviewPanel {
    fn default() -> Self {
        Self {
            tab: WorkbenchTab::Assignment,
            target: ReviewTarget::Current,
            state: LoadState::Idle,
            token: 0,
            review: None,
            diff: DiffState::default(),
            stale: false,
            loaded_commit: None,
            selected_file: None,
            file_list_width: 190.0,
            file_resize_start: None,
            history: HistoryPanel::default(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct NodeDrag {
    pub node: i64,
    pub start: [f32; 2],
}

#[derive(Clone, Debug)]
pub(crate) struct OutputLine {
    pub seq: u64,
    pub text: String,
}

#[derive(Clone, Debug)]
pub(crate) struct PendingSave {
    pub token: u64,
    pub prompt: String,
}

pub(crate) struct NodeDraft {
    pub edit: TextEdit,
    pub preview: bool,
    pub baseline: String,
    pub pending: Option<PendingSave>,
    pub error: Option<String>,
    pub notice: Option<String>,
}

impl NodeDraft {
    pub(crate) fn new(prompt: String) -> Self {
        Self {
            edit: TextEdit::new(prompt.clone()),
            preview: false,
            baseline: prompt,
            pending: None,
            error: None,
            notice: None,
        }
    }

    pub fn dirty(&self) -> bool {
        self.edit.text != self.baseline
    }
}

pub(crate) struct State {
    pub mode: Mode,
    pub graph: Graph,
    pub camera: canvas::Camera,
    pub selected: Option<i64>,
    pub preview_node: Option<i64>,
    pub workbench_open: bool,
    pub workbench_expanded: bool,
    pub pan_start: Option<[f32; 2]>,
    pub node_drag: Option<NodeDrag>,
    pub space_held: bool,
    pub streaming: bool,
    pub output: Vec<OutputLine>,
    pub output_seq: u64,
    pub next_output_at: f64,
    pub backend: Option<Backend>,
    pub backend_state: LoadState,
    pub backend_detail: String,
    pub sync_error: Option<String>,
    pub next_sync_at: f64,
    pub project_open_error: Option<String>,
    pub drop_hover: bool,
    pub workflow_help_open: bool,
    pub workflow_help_return: Option<String>,
    pub prompt_notice: Option<String>,
    pub copy_notice: Option<(String, f64)>,
    pub project_path: Option<PathBuf>,
    pub workflows: Vec<Workflow>,
    pub workflows_state: LoadState,
    pub workflows_token: u64,
    pub selected_workflow: Option<i64>,
    pub source_graph: Option<WorkflowGraph>,
    pub graph_state: LoadState,
    pub graph_token: u64,
    pub drafts: BTreeMap<(i64, i64), NodeDraft>,
    pub assignment_token: u64,
    pub agent: Option<PathBuf>,
    pub runs: BTreeMap<i64, RunStatus>,
    pub runs_state: LoadState,
    pub runs_token: u64,
    pub run_status_state: LoadState,
    pub run_status_token: u64,
    pub run_error: Option<String>,
    pub run_notice: Option<String>,
    pub run_log: ActivityLog,
    pub expanded_tools: BTreeSet<u64>,
    pub run_output_at_bottom: bool,
    pub run_cursor: u64,
    pub run_gap: bool,
    pub run_events_token: u64,
    pub run_events_pending: bool,
    pub next_run_poll: f64,
    pub continuation: Option<(i64, ContinuationAssessment)>,
    pub continuation_token: u64,
    pub follow_up: TextEdit,
    pub review: ReviewPanel,
    pub review_cache: BTreeMap<(i64, i64), ReviewPanel>,
    pub preferences: RunePreferences,
    pub preferences_error: Option<String>,
    pub preferences_writable: bool,
    pub preferences_path: Option<PathBuf>,
    pub settings_open: bool,
    pub display_fixture: bool,
    pub backend_enabled: bool,
    pub waker: Option<Waker>,
    /// Display scale reported by the current view, for saving window geometry.
    pub scale: std::cell::Cell<f32>,
    pub geometry_dirty: bool,
    pub geometry_saved_at: f64,
    pub now: f64,
    pub menu_flags: std::cell::Cell<Option<(bool, bool, bool)>>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            mode: Mode::Fixture,
            graph: fixture::small_graph(),
            camera: canvas::Camera::default(),
            selected: None,
            preview_node: None,
            workbench_open: false,
            workbench_expanded: false,
            pan_start: None,
            node_drag: None,
            space_held: false,
            streaming: false,
            output: Vec::new(),
            output_seq: 0,
            next_output_at: 0.0,
            backend: None,
            backend_state: LoadState::Idle,
            backend_detail: String::new(),
            sync_error: None,
            next_sync_at: 0.0,
            project_open_error: None,
            drop_hover: false,
            workflow_help_open: false,
            workflow_help_return: None,
            prompt_notice: None,
            copy_notice: None,
            project_path: None,
            workflows: Vec::new(),
            workflows_state: LoadState::Idle,
            workflows_token: 0,
            selected_workflow: None,
            source_graph: None,
            graph_state: LoadState::Idle,
            graph_token: 0,
            drafts: BTreeMap::new(),
            assignment_token: 0,
            agent: None,
            runs: BTreeMap::new(),
            runs_state: LoadState::Idle,
            runs_token: 0,
            run_status_state: LoadState::Idle,
            run_status_token: 0,
            run_error: None,
            run_notice: None,
            run_log: ActivityLog::default(),
            expanded_tools: BTreeSet::new(),
            run_output_at_bottom: true,
            run_cursor: 0,
            run_gap: false,
            run_events_token: 0,
            run_events_pending: false,
            next_run_poll: 0.0,
            continuation: None,
            continuation_token: 0,
            follow_up: TextEdit::new(String::new()),
            review: ReviewPanel::default(),
            review_cache: BTreeMap::new(),
            preferences: RunePreferences::default(),
            preferences_error: None,
            preferences_writable: true,
            preferences_path: None,
            settings_open: false,
            display_fixture: false,
            backend_enabled: true,
            waker: None,
            scale: std::cell::Cell::new(1.0),
            geometry_dirty: false,
            geometry_saved_at: 0.0,
            now: 0.0,
            menu_flags: std::cell::Cell::new(None),
        }
    }
}

impl State {
    pub fn draft_for(&self, node_id: i64) -> Option<&NodeDraft> {
        let workflow = self.selected_workflow?;
        self.drafts.get(&(workflow, node_id))
    }

    pub fn ensure_draft_for(&mut self, node_id: i64) {
        let Some(workflow) = self.selected_workflow else {
            return;
        };
        let Some(prompt) = self
            .source_graph
            .as_ref()
            .and_then(|graph| graph.nodes.iter().find(|node| node.id == node_id))
            .map(|node| node.prompt.clone())
        else {
            return;
        };
        self.drafts
            .entry((workflow, node_id))
            .or_insert_with(|| NodeDraft::new(prompt));
    }

    pub fn assignment_edit_mut(&mut self) -> &mut TextEdit {
        let key = (
            self.selected_workflow.unwrap_or_default(),
            self.selected.unwrap_or_default(),
        );
        &mut self
            .drafts
            .entry(key)
            .or_insert_with(|| NodeDraft::new(String::new()))
            .edit
    }

    pub fn load_workflows(&mut self) {
        self.load_workflows_inner(false);
    }

    pub fn refresh_workflows_quietly(&mut self) {
        self.load_workflows_inner(true);
    }

    fn load_workflows_inner(&mut self, quiet: bool) {
        let Some(project) = self.project_path.clone() else {
            return;
        };
        self.workflows_token += 1;
        let token = self.workflows_token;
        if !quiet {
            self.workflows_state = LoadState::Loading;
        }
        if let Some(backend) = &self.backend {
            backend.send(Command::ListWorkflows {
                token,
                project,
                quiet,
            });
        } else if !quiet {
            self.workflows_state = LoadState::Failed("Backend is not available".into());
        }
    }

    /// Requests an immediate non-blocking refresh of the visible state.
    ///
    /// This is the same path the app polls automatically; manual refresh remains
    /// available and uses the explicit loading state.
    pub fn sync_now(&mut self) {
        if self.mode != Mode::Backend || self.project_path.is_none() || self.display_fixture {
            return;
        }
        match self.backend_state {
            LoadState::Failed(_) | LoadState::Idle => self.retry_backend(),
            LoadState::Ready => {
                self.refresh_workflows_quietly();
                if self.selected_workflow.is_some() {
                    self.load_graph_inner(true);
                    self.load_workflow_runs_inner(true);
                }
            }
            LoadState::Loading => {}
        }
    }

    /// Applies an agent open request. Paths are validated by the control layer
    /// and again here so direct callers cannot select a missing folder.
    pub fn apply_open_request(&mut self, project: Option<PathBuf>, workflow: Option<i64>) {
        self.settings_open = false;
        self.workflow_help_open = false;
        self.prompt_notice = None;
        if let Some(project) = project {
            let path = match std::fs::canonicalize(&project) {
                Ok(path) if path.is_dir() => path,
                Ok(_) => {
                    self.project_open_error =
                        Some(format!("{} is not a folder", project.display()));
                    return;
                }
                Err(error) => {
                    self.project_open_error = Some(format!(
                        "Couldn’t open {}: {error}. Choose an existing folder.",
                        project.display()
                    ));
                    return;
                }
            };
            self.project_open_error = None;
            let same_project = self.project_path.as_ref() == Some(&path);
            self.set_sidebar_open(true);
            if same_project {
                if matches!(self.backend_state, LoadState::Failed(_) | LoadState::Idle) {
                    self.retry_backend();
                }
                self.refresh_workflows_quietly();
            } else {
                self.add_project(path);
            }
            if let Some(workflow_id) = workflow {
                if self.selected_workflow == Some(workflow_id) {
                    self.load_graph_inner(true);
                } else {
                    self.select_workflow(workflow_id);
                }
            }
        }
        self.next_sync_at = 0.0;
    }

    pub fn open_workflow_help(&mut self, return_key: &str) {
        self.workflow_help_open = true;
        self.workflow_help_return = Some(return_key.to_owned());
        self.prompt_notice = None;
    }

    pub fn close_workflow_help(&mut self) {
        self.workflow_help_open = false;
        self.prompt_notice = None;
    }

    pub fn select_workflow(&mut self, workflow_id: i64) {
        if self.selected_workflow == Some(workflow_id)
            && matches!(self.graph_state, LoadState::Ready | LoadState::Loading)
        {
            return;
        }
        if self.selected_workflow != Some(workflow_id) {
            self.stash_review();
            self.review = ReviewPanel::default();
            self.graph = Graph::default();
            self.source_graph = None;
            self.selected = None;
            self.preview_node = None;
            self.workbench_open = false;
        }
        self.selected_workflow = Some(workflow_id);
        self.preferences.selected_workflow = Some(workflow_id);
        if let Some(project) = &self.project_path {
            self.preferences.selected_project = Some(project.clone());
        }
        self.save_preferences();
        self.load_graph();
    }

    pub fn load_graph(&mut self) {
        self.load_graph_inner(false);
    }

    fn load_graph_inner(&mut self, quiet: bool) {
        let Some(workflow_id) = self.selected_workflow else {
            return;
        };
        self.graph_token += 1;
        let token = self.graph_token;
        if !quiet {
            self.graph_state = LoadState::Loading;
        }
        if let Some(backend) = &self.backend {
            backend.send(Command::LoadGraph {
                token,
                workflow_id,
                quiet,
            });
        } else if !quiet {
            self.graph_state = LoadState::Failed("Backend is not available".into());
        }
    }

    pub fn save_assignment(&mut self) {
        let (Some(workflow_id), Some(node_id)) = (self.selected_workflow, self.selected) else {
            return;
        };
        let Some(draft) = self.drafts.get(&(workflow_id, node_id)) else {
            return;
        };
        if draft.pending.is_some() {
            return;
        }
        let prompt = draft.edit.text.clone();
        let expected_prompt = draft.baseline.clone();
        self.assignment_token += 1;
        let token = self.assignment_token;
        if let Some(draft) = self.drafts.get_mut(&(workflow_id, node_id)) {
            draft.pending = Some(PendingSave {
                token,
                prompt: prompt.clone(),
            });
            draft.error = None;
            draft.notice = None;
        }
        if let Some(backend) = &self.backend {
            backend.send(Command::SaveAssignment {
                token,
                workflow_id,
                node_id,
                prompt,
                expected_prompt,
            });
        } else if let Some(draft) = self.drafts.get_mut(&(workflow_id, node_id)) {
            draft.pending = None;
            draft.error = Some("Backend is not available".into());
        }
    }

    pub fn discard_assignment(&mut self) {
        let (Some(workflow_id), Some(node_id)) = (self.selected_workflow, self.selected) else {
            return;
        };
        let Some(prompt) = self
            .source_graph
            .as_ref()
            .and_then(|graph| graph.nodes.iter().find(|node| node.id == node_id))
            .map(|node| node.prompt.clone())
        else {
            return;
        };
        if let Some(draft) = self.drafts.get_mut(&(workflow_id, node_id)) {
            if draft.pending.is_some() {
                return;
            }
            draft.baseline = prompt.clone();
            draft.edit.replace_text(prompt);
            draft.error = None;
            draft.notice = Some("Loaded saved assignment".into());
        }
    }

    pub fn set_appearance(&mut self, appearance: Appearance) {
        self.preferences.appearance = appearance;
        self.save_preferences();
    }

    pub fn set_reduced_motion(&mut self, reduced: bool) {
        self.preferences.reduced_motion = reduced;
        self.save_preferences();
    }

    pub fn observe_window_geometry(&mut self, rect: [f32; 4]) {
        if rune::is_headless() {
            return;
        }
        let scale = self.scale.get();
        let geometry = crate::preferences::WindowGeometry {
            x: rect[0],
            y: rect[1],
            width: rect[2],
            height: rect[3],
            scale,
        };
        if !geometry.valid() {
            return;
        }
        if self.preferences.window_geometry != Some(geometry) {
            self.preferences.window_geometry = Some(geometry);
            self.geometry_dirty = true;
        }
    }

    pub fn tick_preferences(&mut self, clock: f64) {
        self.now = clock;
        if self.geometry_dirty && clock - self.geometry_saved_at >= 0.5 {
            self.save_preferences();
            self.geometry_dirty = false;
            self.geometry_saved_at = clock;
        }
    }

    /// Shows short-lived confirmation beside copy actions.
    pub fn note_copied(&mut self, message: impl Into<String>) {
        self.copy_notice = Some((message.into(), self.now));
    }

    pub fn copy_notice(&self) -> Option<&str> {
        self.copy_notice
            .as_ref()
            .filter(|(_, at)| self.now - at < 2.5)
            .map(|(message, _)| message.as_str())
    }

    pub fn expire_copy_notice(&mut self, clock: f64) -> bool {
        if self
            .copy_notice
            .as_ref()
            .is_some_and(|(_, at)| clock - at >= 2.5)
        {
            self.copy_notice = None;
            true
        } else {
            false
        }
    }

    pub fn flush_preferences(&mut self) {
        if self.geometry_dirty {
            self.save_preferences();
            self.geometry_dirty = false;
            self.geometry_saved_at = self.now;
        }
    }

    pub fn set_sidebar_open(&mut self, open: bool) {
        self.preferences.sidebar_open = open;
        self.save_preferences();
    }

    pub fn toggle_sidebar(&mut self) {
        let open = !self.preferences.sidebar_open;
        self.set_sidebar_open(open);
    }

    pub fn set_interface_zoom(&mut self, zoom: f32) {
        self.preferences.interface_zoom = zoom.clamp(
            crate::preferences::INTERFACE_ZOOM_LEVELS[0],
            crate::preferences::INTERFACE_ZOOM_LEVELS
                [crate::preferences::INTERFACE_ZOOM_LEVELS.len() - 1],
        );
        self.save_preferences();
    }

    pub(crate) fn save_preferences(&mut self) {
        if !self.preferences_writable || rune::is_headless() {
            return;
        }
        let Some(path) = self.preferences_path.clone() else {
            return;
        };
        match preferences::save(&self.preferences, &path) {
            Ok(()) => self.preferences_error = None,
            Err(error) => self.preferences_error = Some(error),
        }
    }

    pub fn add_project(&mut self, path: PathBuf) {
        match self.open_project(path) {
            Ok(()) => {
                self.project_open_error = None;
                self.preferences_error = None;
            }
            Err(error) => {
                self.project_open_error = Some(error);
            }
        }
    }

    /// Validates and opens one project folder. Shared by the dialog, folder drop,
    /// and agent open requests; it never creates a workflow or initializes a repo.
    pub fn open_project(&mut self, path: PathBuf) -> Result<(), String> {
        let path = match std::fs::canonicalize(&path) {
            Ok(path) if path.is_dir() => path,
            Ok(_) => return Err(format!("{} is not a folder", path.display())),
            Err(error) => {
                return Err(format!(
                    "Couldn’t open {}: {error}. Choose an existing folder.",
                    path.display()
                ));
            }
        };
        let same_project = self.project_path.as_ref() == Some(&path);
        if !self.preferences.projects.contains(&path) {
            self.preferences.projects.push(path.clone());
        }
        self.preferences.selected_project = Some(path.clone());
        self.preferences.sidebar_open = true;
        if !same_project {
            self.preferences.selected_workflow = None;
        }
        self.save_preferences();
        self.select_project(path);
        Ok(())
    }

    /// Accepts exactly one dropped folder. Mixed or multiple drops are rejected
    /// rather than choosing one silently.
    pub fn drop_project(&mut self, files: &[PathBuf]) {
        if files.len() != 1 {
            let message = if files.is_empty() {
                "Drop one project folder here".to_owned()
            } else {
                format!(
                    "Drop one project folder at a time; {} items were dropped",
                    files.len()
                )
            };
            self.project_open_error = Some(message);
            return;
        }
        self.add_project(files[0].clone());
    }

    pub fn remove_project(&mut self, path: &std::path::Path) {
        self.preferences.projects.retain(|project| project != path);
        if self.preferences.selected_project.as_deref() == Some(path) {
            self.preferences.selected_project = self.preferences.projects.first().cloned();
            self.preferences.selected_workflow = None;
            if let Some(next) = self.preferences.projects.first().cloned() {
                self.save_preferences();
                self.select_project(next);
                return;
            }
            self.project_path = None;
            self.graph = Graph::default();
            self.workflows.clear();
            self.workflows_state = LoadState::Idle;
            self.graph_state = LoadState::Idle;
            self.selected_workflow = None;
            self.selected = None;
            self.workbench_open = false;
        }
        self.save_preferences();
    }

    pub fn select_project(&mut self, path: PathBuf) {
        self.project_open_error = None;
        if self.project_path.as_ref() == Some(&path) && self.mode == Mode::Backend {
            self.preferences.selected_project = Some(path);
            self.save_preferences();
            self.refresh_workflows_quietly();
            return;
        }
        self.stash_review();
        self.mode = Mode::Backend;
        self.display_fixture = false;
        self.project_path = Some(path.clone());
        self.preferences.selected_project = Some(path);
        self.preferences.selected_workflow = None;
        self.save_preferences();
        self.graph = Graph::default();
        self.source_graph = None;
        self.workflows.clear();
        self.workflows_state = LoadState::Idle;
        self.graph_state = LoadState::Idle;
        self.selected_workflow = None;
        self.selected = None;
        self.preview_node = None;
        self.workbench_open = false;
        self.review = ReviewPanel::default();
        let had_backend = self.backend.is_some();
        crate::startup::start_backend(self);
        if had_backend {
            self.retry_backend();
        }
    }

    pub fn retry_backend(&mut self) {
        self.backend_state = LoadState::Loading;
        if self.backend.is_none() {
            crate::startup::start_backend(self);
        }
        if let Some(backend) = &self.backend {
            backend.send(Command::Connect);
        } else {
            self.backend_state = LoadState::Failed("Backend is not available".into());
        }
    }

    /// A node switch drops in-flight replies for the old node. If the cached
    /// panel was left in a loading state, issue fresh requests on return so it
    /// cannot stay "Loading" forever.
    fn resume_review_requests(&mut self) {
        if self.review.state == LoadState::Loading {
            let target = self.review.target.clone();
            self.request_review(target);
        }
        if self.review.history.runs_state == LoadState::Loading {
            self.load_history_runs(true);
        }
        if self.review.history.revisions_state == LoadState::Loading {
            self.load_history_revisions(true);
        }
    }

    fn stash_review(&mut self) {
        if let (Some(workflow_id), Some(node_id)) = (self.selected_workflow, self.selected) {
            let panel = std::mem::take(&mut self.review);
            self.review_cache.insert((workflow_id, node_id), panel);
        }
    }

    fn restore_review(&mut self, node_id: i64) {
        self.review = self
            .selected_workflow
            .and_then(|workflow_id| self.review_cache.remove(&(workflow_id, node_id)))
            .unwrap_or_default();
    }

    pub fn set_workbench_tab(&mut self, tab: WorkbenchTab) {
        if self.review.tab != tab {
            self.copy_notice = None;
        }
        self.review.tab = tab;
        if tab == WorkbenchTab::Changes && self.review.state == LoadState::Idle {
            self.review_current();
        }
        if tab == WorkbenchTab::History && self.review.history.runs_state == LoadState::Idle {
            self.load_history_runs(true);
        }
        if tab == WorkbenchTab::Output {
            self.load_run_transcript();
        }
    }

    pub fn review_current(&mut self) {
        self.request_review(ReviewTarget::Current);
    }

    pub fn review_latest_run(&mut self) {
        let Some(run) = self.selected_run() else {
            return;
        };
        if matches!(run.run.snapshot, SnapshotState::Saved { .. }) {
            self.review_run(run.run.id);
        }
    }

    pub fn review_run(&mut self, run_id: i64) {
        self.request_review(ReviewTarget::Run(run_id));
    }

    pub fn review_revision(&mut self, commit_id: String) {
        self.request_review(ReviewTarget::Revision(commit_id));
    }

    pub fn set_history_kind(&mut self, kind: HistoryKind) {
        if self.review.history.kind != kind {
            self.review.history.notice = None;
            self.review.history.notice_error = false;
        }
        self.review.history.kind = kind;
        match kind {
            HistoryKind::Runs if self.review.history.runs_state == LoadState::Idle => {
                self.load_history_runs(true);
            }
            HistoryKind::Revisions if self.review.history.revisions_state == LoadState::Idle => {
                self.load_history_revisions(true);
            }
            _ => {}
        }
    }

    pub fn load_history_runs(&mut self, reset: bool) {
        let (Some(workflow_id), Some(node_id)) = (self.selected_workflow, self.selected) else {
            return;
        };
        let history = &mut self.review.history;
        if !reset && history.runs_cursor.is_none() {
            return;
        }
        history.token += 1;
        let token = history.token;
        let cursor = if reset {
            // Keep loaded rows and the selection visible while refreshing; a
            // failed refresh must not destroy usable history.
            history.runs_cursor = None;
            history.runs_state = LoadState::Loading;
            history.notice = if history.runs.is_empty() {
                None
            } else {
                Some("Refreshing runs…".into())
            };
            history.notice_error = false;
            None
        } else {
            history.notice = Some("Loading older runs…".into());
            history.notice_error = false;
            history.runs_cursor.clone()
        };
        if let Some(backend) = &self.backend {
            backend.send(Command::HistoryRuns {
                token,
                workflow_id,
                node_id,
                page_size: 30,
                cursor,
            });
        } else if history.runs.is_empty() {
            history.runs_state = LoadState::Failed("Backend is not available".into());
            history.notice = None;
            history.notice_error = false;
        } else {
            history.runs_state = LoadState::Ready;
            history.notice = Some("Refresh failed · Backend is not available".into());
            history.notice_error = true;
        }
    }

    pub fn load_history_revisions(&mut self, reset: bool) {
        let (Some(workflow_id), Some(node_id)) = (self.selected_workflow, self.selected) else {
            return;
        };
        let history = &mut self.review.history;
        if !reset && history.revisions_cursor.is_none() {
            return;
        }
        history.token += 1;
        let token = history.token;
        let cursor = if reset {
            history.revisions_cursor = None;
            history.revisions_state = LoadState::Loading;
            history.notice = if history.revisions.is_empty() {
                None
            } else {
                Some("Refreshing native revisions…".into())
            };
            history.notice_error = false;
            None
        } else {
            history.notice = Some("Loading older revisions…".into());
            history.notice_error = false;
            history.revisions_cursor.clone()
        };
        if let Some(backend) = &self.backend {
            backend.send(Command::HistoryRevisions {
                token,
                workflow_id,
                node_id,
                page_size: 30,
                cursor,
            });
        } else if history.revisions.is_empty() {
            history.revisions_state = LoadState::Failed("Backend is not available".into());
            history.notice = None;
            history.notice_error = false;
        } else {
            history.revisions_state = LoadState::Ready;
            history.notice = Some("Refresh failed · Backend is not available".into());
            history.notice_error = true;
        }
    }

    pub fn review_history_run(&mut self) {
        let Some(run_id) = self.review.history.selected_run else {
            return;
        };
        let saved = self
            .review
            .history
            .runs
            .iter()
            .find(|run| run.run.id == run_id)
            .is_some_and(|run| matches!(run.run.snapshot, SnapshotState::Saved { .. }));
        if saved {
            self.review_run(run_id);
            self.review.tab = WorkbenchTab::Changes;
        }
    }

    pub fn review_history_revision(&mut self) {
        let Some(commit_id) = self.review.history.selected_revision.clone() else {
            return;
        };
        self.review_revision(commit_id);
        self.review.tab = WorkbenchTab::Changes;
    }

    pub(crate) fn request_review(&mut self, target: ReviewTarget) {
        let (Some(workflow_id), Some(node_id)) = (self.selected_workflow, self.selected) else {
            return;
        };
        self.review.token += 1;
        let token = self.review.token;
        self.review.target = target.clone();
        self.review.state = LoadState::Loading;
        self.review.review = None;
        self.review.loaded_commit = None;
        self.review.stale = false;
        self.review.selected_file = None;
        self.review.diff.set_files(Vec::new());
        let Some(backend) = &self.backend else {
            self.review.state = LoadState::Failed("Backend is not available".into());
            return;
        };
        match target {
            ReviewTarget::Current => backend.send(Command::ReviewNode {
                token,
                workflow_id,
                node_id,
            }),
            ReviewTarget::Run(run_id) => backend.send(Command::ReviewRun {
                token,
                workflow_id,
                node_id,
                run_id,
            }),
            ReviewTarget::Revision(commit_id) => backend.send(Command::ReviewRevision {
                token,
                workflow_id,
                node_id,
                commit_id,
            }),
        }
    }

    pub fn scroll_review_to_file(&mut self, index: usize, tx: &mut rune::Tx) {
        self.review.selected_file = Some(index);
        self.review.diff.scroll_to_file(index, tx);
    }

    fn refresh_review_staleness(&mut self) {
        if self.review.target != ReviewTarget::Current {
            return;
        }
        let Some(loaded) = self.review.loaded_commit.as_deref() else {
            return;
        };
        if let Some(run) = self.selected_run()
            && let SnapshotState::Saved { revision } = &run.run.snapshot
            && revision.commit_id != loaded
        {
            self.review.stale = true;
        }
    }

    pub fn select_remote_node(&mut self, node_id: i64) {
        if self.selected != Some(node_id) {
            self.stash_review();
            self.copy_notice = None;
            self.run_log = ActivityLog::default();
            self.expanded_tools.clear();
            self.run_output_at_bottom = true;
            self.run_cursor = 0;
            self.run_gap = false;
            self.run_error = None;
            self.run_notice = None;
            self.continuation = None;
            self.selected = Some(node_id);
            self.restore_review(node_id);
            self.resume_review_requests();
        }
        self.selected = Some(node_id);
        self.preview_node = None;
        self.ensure_draft_for(node_id);
        let Some(run) = self.selected_run() else {
            return;
        };
        let run_id = run.run.id;
        let stopped = !crate::activity::run_is_active(run);
        let has_session = run.run.session_id.is_some();
        if stopped {
            if has_session {
                self.request_continuation(run_id);
            }
            self.load_run_transcript();
        } else {
            self.next_run_poll = 0.0;
            self.request_run_events();
        }
    }

    pub fn selected_run(&self) -> Option<&RunStatus> {
        let node_id = self.selected?;
        self.runs.get(&node_id)
    }

    pub fn load_workflow_runs(&mut self) {
        self.load_workflow_runs_inner(false);
    }

    fn load_workflow_runs_inner(&mut self, quiet: bool) {
        let Some(workflow_id) = self.selected_workflow else {
            return;
        };
        self.runs_token += 1;
        let token = self.runs_token;
        if !quiet {
            self.runs_state = LoadState::Loading;
        }
        if let Some(backend) = &self.backend {
            backend.send(Command::WorkflowRuns {
                token,
                workflow_id,
                quiet,
            });
        } else if !quiet {
            self.runs_state = LoadState::Failed("Backend is not available".into());
        }
    }

    pub fn request_run_events(&mut self) {
        if self.run_events_pending {
            return;
        }
        let Some(run) = self.selected_run() else {
            return;
        };
        if !crate::activity::run_is_active(run) {
            return;
        }
        self.request_run_events_from(self.run_cursor);
    }

    /// Reload a stopped run's bounded transcript after switching away from and
    /// back to its node, or when Output is opened on an existing run. This is
    /// read-only and never starts or continues a run.
    pub fn load_run_transcript(&mut self) {
        if self.run_events_pending || !self.run_log.blocks().is_empty() {
            return;
        }
        if self.selected_run().is_none() {
            return;
        }
        self.request_run_events_from(0);
    }

    fn request_run_events_from(&mut self, after: u64) {
        let Some(run) = self.selected_run() else {
            return;
        };
        let run_id = run.run.id;
        self.run_events_token += 1;
        let token = self.run_events_token;
        self.run_events_pending = true;
        if let Some(backend) = &self.backend {
            backend.send(Command::RunEvents {
                token,
                run_id,
                after,
            });
        } else {
            self.run_events_pending = false;
        }
    }

    pub fn request_continuation(&mut self, run_id: i64) {
        if self
            .continuation
            .as_ref()
            .is_some_and(|(id, _)| *id == run_id)
        {
            return;
        }
        self.continuation_token += 1;
        let token = self.continuation_token;
        if let Some(backend) = &self.backend {
            backend.send(Command::AssessContinuation { token, run_id });
        }
    }

    pub fn start_run(&mut self) {
        let (Some(workflow_id), Some(node_id)) = (self.selected_workflow, self.selected) else {
            return;
        };
        let Some(executable) = self.agent.clone() else {
            self.run_error =
                Some("Set --agent /absolute/path/to/acp-executable to start a run".into());
            return;
        };
        self.run_status_state = LoadState::Loading;
        self.run_error = None;
        self.run_notice = None;
        self.run_log = ActivityLog::default();
        self.expanded_tools.clear();
        self.run_output_at_bottom = true;
        self.run_cursor = 0;
        self.run_gap = false;
        self.continuation = None;
        self.run_status_token += 1;
        let token = self.run_status_token;
        if let Some(backend) = &self.backend {
            backend.send(Command::StartRun {
                token,
                workflow_id,
                node_id,
                executable,
            });
        }
    }

    pub fn continue_run(&mut self) {
        let Some(run) = self.selected_run() else {
            return;
        };
        if crate::activity::run_is_active(run) {
            return;
        }
        let run_id = run.run.id;
        let prompt = self.follow_up.text.trim().to_owned();
        if prompt.is_empty() {
            self.run_error = Some("Enter a follow-up before continuing the session".into());
            return;
        }
        self.run_status_state = LoadState::Loading;
        self.run_error = None;
        self.run_notice = None;
        self.run_status_token += 1;
        let token = self.run_status_token;
        if let Some(backend) = &self.backend {
            backend.send(Command::ContinueRun {
                token,
                run_id,
                prompt,
            });
        }
    }

    pub fn cancel_run(&mut self) {
        let Some(run) = self.selected_run() else {
            return;
        };
        if !crate::activity::run_is_active(run) {
            return;
        }
        let run_id = run.run.id;
        self.run_status_state = LoadState::Loading;
        self.run_error = None;
        self.run_status_token += 1;
        let token = self.run_status_token;
        if let Some(backend) = &self.backend {
            backend.send(Command::CancelRun { token, run_id });
        }
    }

    pub fn reply_permission(&mut self, permission_id: u64, option_id: String) {
        let Some(run) = self.selected_run() else {
            return;
        };
        let run_id = run.run.id;
        self.run_status_state = LoadState::Loading;
        self.run_error = None;
        self.run_status_token += 1;
        let token = self.run_status_token;
        if let Some(backend) = &self.backend {
            backend.send(Command::ReplyPermission {
                token,
                run_id,
                permission_id,
                option_id,
            });
        }
    }

    pub fn follow_up_edit_mut(&mut self) -> &mut TextEdit {
        &mut self.follow_up
    }

    fn sync_node_statuses(&mut self) {
        for node in &mut self.graph.nodes {
            if let Some(run) = self.runs.get(&node.id) {
                node.status = crate::activity::status_for_run(&run.run.state);
            } else {
                node.status = fixture::Status::Unknown;
            }
        }
        self.refresh_review_staleness();
        self.refresh_history_freshness();
    }

    fn refresh_history_freshness(&mut self) {
        let Some((run_id, snapshot_commit)) = self.selected_run().map(|run| {
            (
                run.run.id,
                match &run.run.snapshot {
                    SnapshotState::Saved { revision } => Some(revision.commit_id.clone()),
                    _ => None,
                },
            )
        }) else {
            return;
        };
        let history = &mut self.review.history;
        if history.runs_state == LoadState::Ready
            && history.runs.iter().all(|saved| saved.run.id != run_id)
        {
            history.fresh = true;
        }
        if history.revisions_state == LoadState::Ready
            && let Some(commit_id) = snapshot_commit
            && history
                .revisions
                .iter()
                .all(|entry| entry.evolution.revision.commit_id != commit_id)
        {
            history.fresh = true;
        }
    }

    fn reconcile_graph(&mut self, workflow_id: i64, graph: WorkflowGraph, quiet: bool) {
        if self.source_graph.as_ref() == Some(&graph) {
            self.graph_state = LoadState::Ready;
            self.sync_error = None;
            // The periodic sync already requested run statuses alongside the
            // graph; an explicit graph refresh asks for them here.
            if !quiet {
                self.load_workflow_runs();
            }
            return;
        }
        let had_graph = self.source_graph.is_some();
        let previous = self
            .graph
            .nodes
            .iter()
            .map(|node| (node.id, (node.x, node.y, node.status)))
            .collect::<BTreeMap<_, _>>();
        for node in &graph.nodes {
            if let Some(draft) = self.drafts.get_mut(&(workflow_id, node.id)) {
                if draft.pending.is_some() {
                    continue;
                }
                if !draft.dirty() && draft.baseline != node.prompt {
                    draft.baseline = node.prompt.clone();
                    draft.edit.replace_text(node.prompt.clone());
                    draft.notice = Some("Assignment refreshed from backend".into());
                } else if draft.dirty() && draft.baseline != node.prompt {
                    draft.notice =
                        Some("Saved assignment changed elsewhere; unsaved draft kept".into());
                }
            }
        }
        let mut presented = fixture::from_workflow(&graph);
        // External graph edits must not throw away presentation-only card moves or
        // make already-known run statuses flicker to Unknown on every sync.
        for node in &mut presented.nodes {
            if let Some((x, y, status)) = previous.get(&node.id) {
                node.x = *x;
                node.y = *y;
                node.status = *status;
            }
        }
        self.graph = presented;
        self.source_graph = Some(graph);
        self.graph_state = LoadState::Ready;
        self.sync_error = None;
        if self
            .selected
            .is_some_and(|node_id| self.graph.node(node_id).is_none())
        {
            self.stash_review();
            self.review = ReviewPanel::default();
            self.selected = None;
            self.preview_node = None;
            self.workbench_open = false;
        }
        if let Some(node_id) = self.selected {
            self.ensure_draft_for(node_id);
        }
        if !quiet || !had_graph {
            self.runs.clear();
            self.run_log = ActivityLog::default();
            self.expanded_tools.clear();
            self.run_output_at_bottom = true;
            self.run_cursor = 0;
            self.run_gap = false;
            self.run_events_pending = false;
            self.run_error = None;
            self.run_notice = None;
            self.continuation = None;
            self.load_workflow_runs();
        } else {
            self.load_workflow_runs_inner(true);
        }
    }

    pub(crate) fn apply_backend_update(&mut self, update: Update) {
        match update {
            Update::Connected { database } => {
                self.backend_state = LoadState::Ready;
                self.backend_detail = format!("Connected · {}", database.display());
                self.sync_error = None;
                self.load_workflows();
            }
            Update::Unavailable { message } => {
                self.backend_state = LoadState::Failed(message.clone());
                self.backend_detail = message.clone();
                self.sync_error = Some(message);
            }
            Update::Workflows {
                token,
                project,
                quiet,
                result,
            } => {
                if token != self.workflows_token || self.project_path.as_ref() != Some(&project) {
                    return;
                }
                match result {
                    Ok(workflows) => {
                        self.workflows = workflows;
                        self.workflows_state = LoadState::Ready;
                        self.sync_error = None;
                        if quiet {
                            // A new externally-created workflow is revealed when the
                            // app had nothing selected; an existing selection is never
                            // replaced by a background refresh.
                            if self.selected_workflow.is_none()
                                && let Some(first) =
                                    self.workflows.first().map(|workflow| workflow.id)
                            {
                                self.selected_workflow = Some(first);
                                self.load_graph();
                            }
                            return;
                        }
                        let preferred = self
                            .selected_workflow
                            .filter(|id| self.workflows.iter().any(|workflow| workflow.id == *id))
                            .or_else(|| {
                                startup().workflow.filter(|id| {
                                    self.workflows.iter().any(|workflow| workflow.id == *id)
                                })
                            })
                            .or_else(|| self.workflows.first().map(|workflow| workflow.id));
                        if let Some(workflow_id) = preferred {
                            if self.selected_workflow != Some(workflow_id)
                                || !matches!(
                                    self.graph_state,
                                    LoadState::Ready | LoadState::Loading
                                )
                            {
                                self.selected_workflow = Some(workflow_id);
                                self.load_graph();
                            }
                        } else {
                            self.selected_workflow = None;
                            self.graph = Graph::default();
                            self.graph_state = LoadState::Idle;
                        }
                    }
                    Err(error) => {
                        if quiet {
                            self.sync_error =
                                Some(format!("Live workflow refresh failed: {error}"));
                            if matches!(self.workflows_state, LoadState::Idle | LoadState::Loading)
                            {
                                self.workflows_state = LoadState::Failed(error);
                            }
                        } else {
                            self.workflows_state = LoadState::Failed(error);
                        }
                    }
                }
            }
            Update::Graph {
                token,
                workflow_id,
                quiet,
                result,
            } => {
                if token != self.graph_token || self.selected_workflow != Some(workflow_id) {
                    return;
                }
                match *result {
                    Ok(graph) => self.reconcile_graph(workflow_id, graph, quiet),
                    Err(error) => {
                        if quiet && matches!(self.graph_state, LoadState::Ready) {
                            self.sync_error = Some(format!("Live graph refresh failed: {error}"));
                        } else {
                            self.graph_state = LoadState::Failed(error);
                        }
                    }
                }
            }
            Update::AssignmentSaved {
                token,
                workflow_id,
                node_id,
                result,
            } => {
                let key = (workflow_id, node_id);
                let Some(draft) = self.drafts.get_mut(&key) else {
                    return;
                };
                let Some(pending) = draft.pending.clone() else {
                    return;
                };
                if pending.token != token {
                    return;
                }
                draft.pending = None;
                match result {
                    Ok(node) => {
                        let saved_prompt = node.prompt.clone();
                        let typed_later = draft.edit.text != pending.prompt;
                        draft.baseline = saved_prompt.clone();
                        if !typed_later {
                            draft.edit.replace_text(saved_prompt.clone());
                        }
                        draft.error = None;
                        draft.notice = Some(if typed_later {
                            "Saved; newer edits remain unsaved".into()
                        } else {
                            "Assignment saved".into()
                        });
                        if let Some(card) = self.graph.node_mut(node_id) {
                            let (title, summary) = fixture::prompt_text(&saved_prompt);
                            card.title = title;
                            card.summary = summary;
                            card.assignment = saved_prompt.clone();
                        }
                        if let Some(source) = &mut self.source_graph
                            && let Some(node) =
                                source.nodes.iter_mut().find(|node| node.id == node_id)
                        {
                            node.prompt = saved_prompt;
                        }
                    }
                    Err(error) => draft.error = Some(error),
                }
            }
            Update::WorkflowRuns {
                token,
                workflow_id,
                quiet,
                result,
            } => {
                if token != self.runs_token || self.selected_workflow != Some(workflow_id) {
                    return;
                }
                match *result {
                    Ok(nodes) => {
                        let previous = self
                            .selected
                            .and_then(|node_id| self.runs.get(&node_id))
                            .map(|run| run.run.id);
                        let runs: BTreeMap<_, _> = nodes
                            .into_iter()
                            .filter_map(|node| node.latest.map(|run| (node.node_id, run)))
                            .collect();
                        let next = self
                            .selected
                            .and_then(|node_id| runs.get(&node_id))
                            .map(|run| run.run.id);
                        if next.is_some() && next != previous {
                            // An externally started run replaced the selected node's
                            // prior run: replay its events from the beginning instead
                            // of mixing them into the old transcript.
                            self.run_log = ActivityLog::default();
                            self.expanded_tools.clear();
                            self.run_output_at_bottom = true;
                            self.run_cursor = 0;
                            self.run_gap = false;
                            self.run_events_pending = false;
                            self.run_error = None;
                            self.run_notice = None;
                            self.continuation = None;
                        }
                        self.runs_state = LoadState::Ready;
                        self.runs = runs;
                        self.sync_error = None;
                        self.sync_node_statuses();
                        if let Some(run) = self.selected_run() {
                            let run_id = run.run.id;
                            let stopped = !crate::activity::run_is_active(run);
                            let has_session = run.run.session_id.is_some();
                            if stopped && has_session {
                                self.request_continuation(run_id);
                            }
                        }
                    }
                    Err(error) => {
                        if quiet && matches!(self.runs_state, LoadState::Ready) {
                            self.sync_error = Some(format!("Live run refresh failed: {error}"));
                        } else {
                            self.runs_state = LoadState::Failed(error);
                        }
                    }
                }
            }
            Update::RunStatus { token, result } => {
                if token != self.run_status_token {
                    return;
                }
                match *result {
                    Ok(status) => {
                        self.run_status_state = LoadState::Ready;
                        self.run_error = None;
                        self.run_notice = Some("Run status updated".into());
                        self.runs.insert(status.run.node_id, status);
                        self.sync_node_statuses();
                    }
                    Err(error) => {
                        self.run_status_state = LoadState::Failed(error.clone());
                        self.run_error = Some(error);
                    }
                }
            }
            Update::RunEvents {
                token,
                run_id,
                result,
            } => {
                if token != self.run_events_token {
                    return;
                }
                self.run_events_pending = false;
                let Some(selected) = self.selected_run() else {
                    return;
                };
                if selected.run.id != run_id {
                    return;
                }
                match *result {
                    Ok(events) => {
                        self.run_status_state = LoadState::Ready;
                        self.run_gap |= events.gap;
                        self.runs.insert(events.status.run.node_id, events.status);
                        self.sync_node_statuses();
                        self.run_log.apply(events.events, events.gap);
                        self.run_cursor = events.next;
                        let stopped = self
                            .selected_run()
                            .is_some_and(|run| !crate::activity::run_is_active(run));
                        let continuation = self
                            .selected_run()
                            .map(|run| (run.run.id, run.run.session_id.is_some()));
                        if stopped {
                            self.run_notice = Some("Run stopped".into());
                            if let Some((run_id, true)) = continuation {
                                self.request_continuation(run_id);
                            }
                        }
                        self.run_error = None;
                    }
                    Err(error) => {
                        self.run_status_state = LoadState::Failed(error.clone());
                        self.run_error = Some(error);
                    }
                }
            }
            Update::Continuation {
                token,
                run_id,
                result,
            } => {
                if token != self.continuation_token {
                    return;
                }
                if let Ok(assessment) = &*result
                    && let ContinuationAssessment::Eligible { run_id: eligible } = assessment
                    && *eligible != run_id
                {
                    return;
                }
                match *result {
                    Ok(assessment) => self.continuation = Some((run_id, assessment)),
                    Err(error) => {
                        self.run_error = Some(error);
                        self.continuation = None;
                    }
                }
            }
            Update::Review {
                token,
                workflow_id,
                node_id,
                target,
                result,
            } => {
                if token != self.review.token
                    || self.selected_workflow != Some(workflow_id)
                    || self.selected != Some(node_id)
                    || target != self.review.target
                {
                    return;
                }
                match *result {
                    Ok(review) => {
                        self.review.diff.set_files(diff::parse(&review.patch));
                        self.review.loaded_commit = Some(review.revision.commit_id.clone());
                        self.review.review = Some(review);
                        self.review.state = LoadState::Ready;
                        self.review.stale = false;
                        self.review.selected_file =
                            (!self.review.diff.files().is_empty()).then_some(0);
                        self.refresh_review_staleness();
                    }
                    Err(error) => {
                        self.review.state = LoadState::Failed(error);
                        self.review.review = None;
                        self.review.loaded_commit = None;
                        self.review.diff.set_files(Vec::new());
                    }
                }
            }
            Update::RunsPage {
                token,
                workflow_id,
                node_id,
                append,
                result,
            } => {
                let history = &mut self.review.history;
                if token != history.token
                    || self.selected_workflow != Some(workflow_id)
                    || self.selected != Some(node_id)
                {
                    return;
                }
                match *result {
                    Ok(page) => {
                        if append {
                            history.runs.extend(page.runs);
                        } else {
                            history.runs = page.runs;
                        }
                        history.runs_cursor = page.next_cursor;
                        history.runs_state = LoadState::Ready;
                        history.notice = None;
                        history.notice_error = false;
                        if !append {
                            history.fresh = false;
                            if !history
                                .runs
                                .iter()
                                .any(|run| Some(run.run.id) == history.selected_run)
                            {
                                history.selected_run = history.runs.first().map(|run| run.run.id);
                            }
                        }
                    }
                    Err(error) => {
                        if append {
                            history.notice = Some(format!("Couldn’t load older runs · {error}"));
                            history.notice_error = true;
                        } else if history.runs.is_empty() {
                            history.runs_cursor = None;
                            history.runs_state = LoadState::Failed(error);
                            history.notice_error = false;
                        } else {
                            history.runs_state = LoadState::Ready;
                            history.notice = Some(format!("Refresh failed · {error}"));
                            history.notice_error = true;
                        }
                    }
                }
            }
            Update::RevisionsPage {
                token,
                workflow_id,
                node_id,
                append,
                result,
            } => {
                let history = &mut self.review.history;
                if token != history.token
                    || self.selected_workflow != Some(workflow_id)
                    || self.selected != Some(node_id)
                {
                    return;
                }
                match *result {
                    Ok(page) => {
                        if append {
                            history.revisions.extend(page.entries);
                        } else {
                            history.revisions = page.entries;
                        }
                        history.revisions_cursor = page.next_cursor;
                        history.revisions_state = LoadState::Ready;
                        history.notice = None;
                        history.notice_error = false;
                        if !append {
                            history.fresh = false;
                            if !history.revisions.iter().any(|entry| {
                                Some(entry.evolution.revision.commit_id.as_str())
                                    == history.selected_revision.as_deref()
                            }) {
                                history.selected_revision = history
                                    .revisions
                                    .first()
                                    .map(|entry| entry.evolution.revision.commit_id.clone());
                            }
                        }
                    }
                    Err(error) => {
                        if append {
                            history.notice =
                                Some(format!("Couldn’t load older revisions · {error}"));
                            history.notice_error = true;
                        } else if history.revisions.is_empty() {
                            history.revisions_cursor = None;
                            history.revisions_state = LoadState::Failed(error);
                            history.notice_error = false;
                        } else {
                            history.revisions_state = LoadState::Ready;
                            history.notice = Some(format!("Refresh failed · {error}"));
                            history.notice_error = true;
                        }
                    }
                }
            }
        }
    }
}
