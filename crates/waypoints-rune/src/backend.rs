//! A small async worker around `waypoints_backend::Client`.
//!
//! The UI never opens the database or invokes jj. Commands carry a request token so
//! stale replies can be dropped after project/workflow/node selection changes.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use waypoints_backend::{Client, Config, NodeRunStatus, RunEvents, RunStatus, RunStatusPage};
use waypoints_core::jj::RevisionReview;
use waypoints_core::run::ContinuationAssessment;
use waypoints_core::storage::{Node, NodeRevisionPage, Workflow, WorkflowGraph};

use rune::Waker;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReviewTarget {
    Current,
    Run(i64),
    Revision(String),
}

pub enum Command {
    Connect,
    ListWorkflows {
        token: u64,
        project: PathBuf,
    },
    LoadGraph {
        token: u64,
        workflow_id: i64,
    },
    SaveAssignment {
        token: u64,
        workflow_id: i64,
        node_id: i64,
        prompt: String,
        expected_prompt: String,
    },
    WorkflowRuns {
        token: u64,
        workflow_id: i64,
    },
    StartRun {
        token: u64,
        workflow_id: i64,
        node_id: i64,
        executable: PathBuf,
    },
    ContinueRun {
        token: u64,
        run_id: i64,
        prompt: String,
    },
    RunEvents {
        token: u64,
        run_id: i64,
        after: u64,
    },
    CancelRun {
        token: u64,
        run_id: i64,
    },
    ReplyPermission {
        token: u64,
        run_id: i64,
        permission_id: u64,
        option_id: String,
    },
    AssessContinuation {
        token: u64,
        run_id: i64,
    },
    ReviewNode {
        token: u64,
        workflow_id: i64,
        node_id: i64,
    },
    ReviewRun {
        token: u64,
        workflow_id: i64,
        node_id: i64,
        run_id: i64,
    },
    ReviewRevision {
        token: u64,
        workflow_id: i64,
        node_id: i64,
        commit_id: String,
    },
    HistoryRuns {
        token: u64,
        workflow_id: i64,
        node_id: i64,
        page_size: usize,
        cursor: Option<String>,
    },
    HistoryRevisions {
        token: u64,
        workflow_id: i64,
        node_id: i64,
        page_size: usize,
        cursor: Option<String>,
    },
}

pub enum Update {
    Connected {
        database: PathBuf,
    },
    Unavailable {
        message: String,
    },
    Workflows {
        token: u64,
        project: PathBuf,
        result: Result<Vec<Workflow>, String>,
    },
    Graph {
        token: u64,
        workflow_id: i64,
        result: Box<Result<WorkflowGraph, String>>,
    },
    AssignmentSaved {
        token: u64,
        workflow_id: i64,
        node_id: i64,
        result: Result<Node, String>,
    },
    WorkflowRuns {
        token: u64,
        workflow_id: i64,
        result: Box<Result<Vec<NodeRunStatus>, String>>,
    },
    RunStatus {
        token: u64,
        result: Box<Result<RunStatus, String>>,
    },
    RunEvents {
        token: u64,
        run_id: i64,
        result: Box<Result<RunEvents, String>>,
    },
    Continuation {
        token: u64,
        run_id: i64,
        result: Box<Result<ContinuationAssessment, String>>,
    },
    Review {
        token: u64,
        workflow_id: i64,
        node_id: i64,
        target: ReviewTarget,
        result: Box<Result<RevisionReview, String>>,
    },
    RunsPage {
        token: u64,
        workflow_id: i64,
        node_id: i64,
        append: bool,
        result: Box<Result<RunStatusPage, String>>,
    },
    RevisionsPage {
        token: u64,
        workflow_id: i64,
        node_id: i64,
        append: bool,
        result: Box<Result<NodeRevisionPage, String>>,
    },
}

pub struct Backend {
    commands: Sender<Command>,
    updates: Receiver<Update>,
}

impl Backend {
    /// Spawns a worker that connects to an already-running backend. It never starts
    /// one, matching the desktop app's current startup policy.
    pub fn spawn(config: Config, waker: Waker) -> Self {
        let (commands, command_rx) = mpsc::channel();
        let (update_tx, updates) = mpsc::channel();
        thread::Builder::new()
            .name("waypoints-rune-backend".into())
            .spawn(move || worker(config, command_rx, update_tx, waker))
            .expect("spawning the Rune backend worker");
        Self { commands, updates }
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    pub fn drain(&self) -> Vec<Update> {
        let mut updates = Vec::new();
        while let Ok(update) = self.updates.try_recv() {
            updates.push(update);
        }
        updates
    }
}

fn worker(config: Config, commands: Receiver<Command>, updates: Sender<Update>, waker: Waker) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            send(
                &updates,
                &waker,
                Update::Unavailable {
                    message: format!("could not create the backend request runtime: {error}"),
                },
            );
            return;
        }
    };

    let mut client: Option<Client> = None;
    while let Ok(command) = commands.recv() {
        if client.is_none() {
            match runtime.block_on(Client::connect(&config)) {
                Ok(connected) => {
                    let database = connected.info().database.clone();
                    client = Some(connected);
                    send(&updates, &waker, Update::Connected { database });
                }
                Err(error) => {
                    send(
                        &updates,
                        &waker,
                        Update::Unavailable {
                            message: error.to_string(),
                        },
                    );
                    continue;
                }
            }
        }
        let Some(client) = client.as_ref() else {
            continue;
        };
        let update = match command {
            Command::Connect => continue,
            Command::ListWorkflows { token, project } => {
                let result = runtime
                    .block_on(client.list_workflows(&project))
                    .map_err(|error| error.to_string());
                Update::Workflows {
                    token,
                    project,
                    result,
                }
            }
            Command::LoadGraph { token, workflow_id } => {
                let result = runtime
                    .block_on(client.workflow_graph(workflow_id))
                    .map_err(|error| error.to_string());
                Update::Graph {
                    token,
                    workflow_id,
                    result: Box::new(result),
                }
            }
            Command::SaveAssignment {
                token,
                workflow_id,
                node_id,
                prompt,
                expected_prompt,
            } => {
                let result = runtime
                    .block_on(client.update_node_checked(
                        workflow_id,
                        node_id,
                        &prompt,
                        &expected_prompt,
                    ))
                    .map_err(|error| error.to_string());
                Update::AssignmentSaved {
                    token,
                    workflow_id,
                    node_id,
                    result,
                }
            }
            Command::WorkflowRuns { token, workflow_id } => {
                let result = runtime
                    .block_on(client.workflow_run_page(workflow_id, 100, None))
                    .map(|page| page.nodes)
                    .map_err(|error| error.to_string());
                Update::WorkflowRuns {
                    token,
                    workflow_id,
                    result: Box::new(result),
                }
            }
            Command::StartRun {
                token,
                workflow_id,
                node_id,
                executable,
            } => {
                let result = runtime
                    .block_on(client.start_run(workflow_id, node_id, &executable))
                    .map_err(|error| error.to_string());
                Update::RunStatus {
                    token,
                    result: Box::new(result),
                }
            }
            Command::ContinueRun {
                token,
                run_id,
                prompt,
            } => {
                let result = runtime
                    .block_on(client.continue_run(run_id, &prompt))
                    .map_err(|error| error.to_string());
                Update::RunStatus {
                    token,
                    result: Box::new(result),
                }
            }
            Command::RunEvents {
                token,
                run_id,
                after,
            } => {
                let result = runtime
                    .block_on(client.run_events(run_id, after))
                    .map_err(|error| error.to_string());
                Update::RunEvents {
                    token,
                    run_id,
                    result: Box::new(result),
                }
            }
            Command::CancelRun { token, run_id } => {
                let result = runtime
                    .block_on(client.cancel_run(run_id))
                    .map_err(|error| error.to_string());
                Update::RunStatus {
                    token,
                    result: Box::new(result),
                }
            }
            Command::ReplyPermission {
                token,
                run_id,
                permission_id,
                option_id,
            } => {
                let result = runtime
                    .block_on(client.respond_to_permission(run_id, permission_id, &option_id))
                    .map_err(|error| error.to_string());
                Update::RunStatus {
                    token,
                    result: Box::new(result),
                }
            }
            Command::AssessContinuation { token, run_id } => {
                let result = runtime
                    .block_on(client.assess_continuation(run_id))
                    .map_err(|error| error.to_string());
                Update::Continuation {
                    token,
                    run_id,
                    result: Box::new(result),
                }
            }
            Command::ReviewNode {
                token,
                workflow_id,
                node_id,
            } => {
                let result = runtime
                    .block_on(client.review_node(workflow_id, node_id))
                    .map_err(|error| error.to_string());
                Update::Review {
                    token,
                    workflow_id,
                    node_id,
                    target: ReviewTarget::Current,
                    result: Box::new(result),
                }
            }
            Command::ReviewRun {
                token,
                workflow_id,
                node_id,
                run_id,
            } => {
                let result = runtime
                    .block_on(client.review_run(run_id))
                    .map_err(|error| error.to_string());
                Update::Review {
                    token,
                    workflow_id,
                    node_id,
                    target: ReviewTarget::Run(run_id),
                    result: Box::new(result),
                }
            }
            Command::ReviewRevision {
                token,
                workflow_id,
                node_id,
                commit_id,
            } => {
                let result = runtime
                    .block_on(client.review_node_revision(workflow_id, node_id, &commit_id))
                    .map_err(|error| error.to_string());
                Update::Review {
                    token,
                    workflow_id,
                    node_id,
                    target: ReviewTarget::Revision(commit_id),
                    result: Box::new(result),
                }
            }
            Command::HistoryRuns {
                token,
                workflow_id,
                node_id,
                page_size,
                cursor,
            } => {
                let append = cursor.is_some();
                let result = runtime
                    .block_on(client.node_run_page(
                        workflow_id,
                        node_id,
                        page_size,
                        cursor.as_deref(),
                    ))
                    .map_err(|error| error.to_string());
                Update::RunsPage {
                    token,
                    workflow_id,
                    node_id,
                    append,
                    result: Box::new(result),
                }
            }
            Command::HistoryRevisions {
                token,
                workflow_id,
                node_id,
                page_size,
                cursor,
            } => {
                let append = cursor.is_some();
                let result = runtime
                    .block_on(client.node_revisions(
                        workflow_id,
                        node_id,
                        page_size,
                        cursor.as_deref(),
                    ))
                    .map_err(|error| error.to_string());
                Update::RevisionsPage {
                    token,
                    workflow_id,
                    node_id,
                    append,
                    result: Box::new(result),
                }
            }
        };
        send(&updates, &waker, update);
    }
}

fn send(updates: &Sender<Update>, waker: &Waker, update: Update) {
    if updates.send(update).is_ok() {
        waker();
    }
}
