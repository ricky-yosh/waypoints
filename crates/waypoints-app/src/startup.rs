//! Startup argument parsing and backend initialization.

use std::path::PathBuf;
use std::sync::OnceLock;

use rune::Waker;
use waypoints_backend::Config;

use crate::backend::{Backend, Command};
use crate::fixture::Graph;
use crate::preferences;
use crate::state::{LoadState, Mode, State};

#[derive(Clone, Debug)]
pub(crate) struct StartupConfig {
    pub(crate) project: Option<PathBuf>,
    pub(crate) workflow: Option<i64>,
    pub(crate) agent: Option<PathBuf>,
}

static STARTUP: OnceLock<StartupConfig> = OnceLock::new();

pub(crate) fn startup() -> &'static StartupConfig {
    STARTUP.get_or_init(|| {
        let mut config = StartupConfig {
            project: None,
            workflow: None,
            agent: std::env::var_os("WAYPOINTS_RUNE_AGENT").map(PathBuf::from),
        };
        let args = rune::extra_args();
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--project" => {
                    index += 1;
                    if let Some(value) = args.get(index) {
                        config.project = Some(PathBuf::from(value));
                    }
                }
                "--workflow" => {
                    index += 1;
                    config.workflow = args.get(index).and_then(|value| value.parse().ok());
                }
                "--agent" => {
                    index += 1;
                    config.agent = args.get(index).map(PathBuf::from);
                }
                flag => {
                    if let Some(value) = flag.strip_prefix("--project=") {
                        config.project = Some(PathBuf::from(value));
                    } else if let Some(value) = flag.strip_prefix("--workflow=") {
                        config.workflow = value.parse().ok();
                    } else if let Some(value) = flag.strip_prefix("--agent=") {
                        config.agent = Some(PathBuf::from(value));
                    }
                }
            }
            index += 1;
        }
        config
    })
}

pub(crate) fn init_state() -> State {
    let mut state = State::default();
    let loaded = preferences::load();
    state.preferences = loaded.preferences;
    state.preferences_error = loaded.error;
    state.preferences_writable = loaded.writable;
    state.preferences_path = loaded.path;
    let config = startup();
    let project = if let Some(project) = &config.project {
        if !state.preferences.projects.contains(project) {
            state.preferences.projects.push(project.clone());
        }
        Some(project.clone())
    } else if let Some(project) = state.preferences.selected_project.clone() {
        Some(project)
    } else {
        state.preferences.projects.first().cloned()
    };
    if let Some(project) = project {
        state.mode = Mode::Backend;
        state.project_path = Some(project.clone());
        state.selected_workflow = config.workflow.or_else(|| {
            (state.preferences.selected_project.as_ref() == Some(&project))
                .then_some(state.preferences.selected_workflow)
                .flatten()
        });
        state.preferences.selected_project = Some(project);
        state.agent = config.agent.clone();
        state.graph = Graph::default();
        state.camera.set_pan([286.0, 44.0]);
        state.save_preferences();
    } else if !rune::is_headless() {
        // A native launch with no project is a real onboarding state, not the
        // fixture canvas. Headless runs keep the fixture for deterministic scenes.
        state.mode = Mode::Backend;
        state.display_fixture = false;
        state.graph = Graph::default();
        state.camera.set_pan([286.0, 44.0]);
    }
    state
}

pub(crate) fn setup(state: &mut State, waker: Waker) {
    state.waker = Some(waker);
    if !state.backend_enabled {
        return;
    }
    start_backend(state);
}

pub(crate) fn start_backend(state: &mut State) {
    if !state.backend_enabled
        || state.backend.is_some()
        || state.mode != Mode::Backend
        || state.project_path.is_none()
    {
        return;
    }
    let Some(waker) = state.waker.clone() else {
        return;
    };
    match Config::default_paths() {
        Ok(config) => {
            state.backend = Some(Backend::spawn(config, waker));
            state.backend_state = LoadState::Loading;
            if let Some(backend) = &state.backend {
                backend.send(Command::Connect);
            }
        }
        Err(error) => {
            state.backend_state = LoadState::Failed(error.to_string());
        }
    }
}
