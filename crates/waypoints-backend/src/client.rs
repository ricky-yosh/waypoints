use std::fs::OpenOptions;
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::Context;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::time::{Instant, sleep, timeout};
use waypoints_core::run::{RecoveryConfirmation, RunRecovery};
use waypoints_core::storage::{
    Connection, DependencyOperation, DependencyRecovery, HistoryStatus, Node, NodeChange,
    UndoBoundaryAcknowledgement, Workflow, WorkflowGraph,
};
use waypoints_core::workflow::{BaseSource, NodeParent};
use waypoints_core::workspace::NodeWorkspace;

use crate::config::sidecar;
use crate::protocol::{MAX_FRAME, PROTOCOL_VERSION, TIMEOUT, read_frame};
use crate::{BackendError, BackendInfo, Config};
use crate::{RunEvents, RunStatus};

/// A client that talks only to the local backend, never directly to its database.
#[derive(Debug)]
pub struct Client {
    config: Config,
    info: BackendInfo,
}

impl Client {
    /// Connects to an existing backend without starting one.
    ///
    /// # Errors
    ///
    /// Returns an error if connection, identity checks, or the readiness request fails.
    pub async fn connect(config: &Config) -> Result<Self, BackendError> {
        let stream = timeout(TIMEOUT, UnixStream::connect(&config.socket))
            .await
            .context("backend connection timed out")?
            .context("connecting to backend")?;
        Ok(Self::from_stream(config, stream).await?)
    }

    /// Reuses a backend or starts the supplied Waypoints executable in the background.
    ///
    /// Only missing or refused sockets trigger startup. A live but unresponsive or
    /// incompatible backend is reported as an error. Concurrent starters are safe:
    /// the server acquires ownership locks before opening the database.
    ///
    /// # Errors
    ///
    /// Returns an error if launching fails or the backend is not ready within five seconds.
    pub async fn connect_or_start(
        config: &Config,
        executable: &Path,
    ) -> Result<Self, BackendError> {
        match timeout(TIMEOUT, UnixStream::connect(&config.socket))
            .await
            .context("backend connection timed out")?
        {
            Ok(stream) => return Ok(Self::from_stream(config, stream).await?),
            Err(error) if unavailable(&error) => {}
            Err(error) => {
                return Err(anyhow::Error::new(error)
                    .context("connecting to backend")
                    .into());
            }
        }

        let log_path = sidecar(&config.socket, ".log");
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&log_path)
            .context("opening backend startup log")?;
        let mut child = Command::new(executable)
            .arg("--database")
            .arg(&config.database)
            .arg("--socket")
            .arg(&config.socket)
            .arg("serve")
            .current_dir("/")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log)
            .process_group(0)
            .spawn()
            .context("starting backend executable")?;
        let deadline = Instant::now() + TIMEOUT;
        loop {
            // Reap a losing starter if another process won the ownership lock.
            let _ = child.try_wait().context("checking backend startup")?;
            match UnixStream::connect(&config.socket).await {
                Ok(stream) => return Ok(Self::from_stream(config, stream).await?),
                Err(error) if unavailable(&error) => {}
                Err(error) => {
                    return Err(anyhow::Error::new(error)
                        .context("connecting to backend")
                        .into());
                }
            }
            if Instant::now() >= deadline {
                return Err(anyhow::anyhow!(
                    "backend did not become ready; inspect {} (another backend may own this database)",
                    log_path.display()
                ).into());
            }
            sleep(std::time::Duration::from_millis(25)).await;
        }
    }

    async fn from_stream(config: &Config, stream: UnixStream) -> anyhow::Result<Self> {
        let info: BackendInfo = exchange(stream, "backend.info", json!({})).await?;
        anyhow::ensure!(
            info.database == config.database && info.protocol_version == PROTOCOL_VERSION,
            "socket belongs to a different database or incompatible backend; refusing to start another server"
        );
        Ok(Self {
            config: config.clone(),
            info,
        })
    }

    /// Returns the identity checked when this client connected.
    pub fn info(&self) -> &BackendInfo {
        &self.info
    }

    /// Inspects the next undo/redo metadata actions across a project's workflows.
    ///
    /// # Errors
    ///
    /// Returns an error for an inaccessible project, unsupported history, or RPC failure.
    pub async fn history_status(&self, project: &Path) -> Result<HistoryStatus, BackendError> {
        let project = project
            .canonicalize()
            .context("resolving project directory")?;
        self.call("history.status", json!({"project_path": project}))
            .await
    }

    /// Undoes the selected pending metadata edit, refusing a stale action ID.
    ///
    /// # Errors
    ///
    /// Returns an error for unavailable/blocked/stale history or RPC failure.
    pub async fn undo(
        &self,
        project: &Path,
        expected_edit_id: i64,
    ) -> Result<HistoryStatus, BackendError> {
        let project = project
            .canonicalize()
            .context("resolving project directory")?;
        self.call(
            "history.undo",
            json!({"project_path": project, "expected_edit_id": expected_edit_id}),
        )
        .await
    }

    /// Redoes the selected pending metadata edit without repeating native side effects.
    ///
    /// # Errors
    ///
    /// Returns an error for unavailable/blocked/stale history or RPC failure.
    pub async fn redo(
        &self,
        project: &Path,
        expected_edit_id: i64,
    ) -> Result<HistoryStatus, BackendError> {
        let project = project
            .canonicalize()
            .context("resolving project directory")?;
        self.call(
            "history.redo",
            json!({"project_path": project, "expected_edit_id": expected_edit_id}),
        )
        .await
    }

    /// Creates a workflow through the backend.
    ///
    /// # Errors
    ///
    /// Returns an error on invalid input, a server error, or a transport failure.
    pub async fn create_workflow(
        &self,
        project: &Path,
        name: &str,
    ) -> Result<Workflow, BackendError> {
        let project = project
            .canonicalize()
            .context("resolving project directory")?;
        self.call(
            "workflow.create",
            json!({"project_path": project, "name": name}),
        )
        .await
    }

    /// Creates a workflow from an explicit bookmark or native revision, without editing code.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid input, failed native selection, or RPC failure.
    pub async fn create_workflow_from(
        &self,
        project: &Path,
        name: &str,
        source: &BaseSource,
    ) -> Result<Workflow, BackendError> {
        let project = project
            .canonicalize()
            .context("resolving project directory")?;
        self.call(
            "workflow.create",
            json!({"project_path":project, "name":name, "base":source}),
        )
        .await
    }

    /// Reads a workflow's saved base without querying native state.
    ///
    /// # Errors
    ///
    /// Returns an error for a missing workflow, unreadable data, or RPC failure.
    pub async fn workflow(&self, workflow_id: i64) -> Result<Workflow, BackendError> {
        self.call("workflow.get", json!({"workflow_id": workflow_id}))
            .await
    }

    /// Selects an initial base for an unbased workflow; never replaces a saved base.
    ///
    /// # Errors
    ///
    /// Returns an error for an existing base, failed native selection, or RPC failure.
    pub async fn set_workflow_base(
        &self,
        workflow_id: i64,
        source: &BaseSource,
    ) -> Result<Workflow, BackendError> {
        self.call(
            "workflow.set_base",
            json!({"workflow_id":workflow_id,"source":source}),
        )
        .await
    }

    /// Lists a project's workflows through the backend.
    ///
    /// # Errors
    ///
    /// Returns an error on invalid input, a server error, or a transport failure.
    pub async fn list_workflows(&self, project: &Path) -> Result<Vec<Workflow>, BackendError> {
        let project = project
            .canonicalize()
            .context("resolving project directory")?;
        self.call("workflow.list", json!({"project_path": project}))
            .await
    }

    /// Reads a workflow and its saved graph in deterministic order.
    ///
    /// # Errors
    ///
    /// Returns an error for a missing workflow, server error, or transport failure.
    pub async fn workflow_graph(&self, workflow_id: i64) -> Result<WorkflowGraph, BackendError> {
        self.call("workflow.graph", json!({"workflow_id": workflow_id}))
            .await
    }

    /// Explicitly attaches legacy prompt-only graph roots to their selected workflow base.
    ///
    /// # Errors
    ///
    /// Returns an error for an incomplete selection, native references, or RPC failure.
    pub async fn adopt_workflow_roots(
        &self,
        workflow_id: i64,
        base_nodes: &[i64],
    ) -> Result<WorkflowGraph, BackendError> {
        self.call(
            "workflow.adopt_roots",
            json!({"workflow_id":workflow_id,"base_nodes":base_nodes}),
        )
        .await
    }

    /// Saves a node and an explicit upstream dependency atomically.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid/unrooted workflow, invalid parent, or RPC failure.
    pub async fn create_node_from(
        &self,
        workflow_id: i64,
        prompt: &str,
        parent: &NodeParent,
    ) -> Result<Node, BackendError> {
        self.call(
            "node.create",
            json!({"workflow_id":workflow_id,"prompt":prompt,"parent":parent}),
        )
        .await
    }

    /// Creates native work from a node's saved dependency without an independent base choice.
    ///
    /// # Errors
    ///
    /// Returns an error for missing/busy upstream work, unsupported parents, or RPC failure.
    pub async fn create_node_change_from_dependency(
        &self,
        workflow_id: i64,
        node_id: i64,
    ) -> Result<NodeChange, BackendError> {
        self.call(
            "node.create_change",
            json!({"workflow_id":workflow_id,"node_id":node_id}),
        )
        .await
    }

    /// Saves a prompt node in an existing workflow.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid input, server errors, or transport failures.
    pub async fn create_node(&self, workflow_id: i64, prompt: &str) -> Result<Node, BackendError> {
        self.call(
            "node.create",
            json!({"workflow_id": workflow_id, "prompt": prompt}),
        )
        .await
    }

    /// Reads a node belonging to the selected workflow.
    ///
    /// # Errors
    ///
    /// Returns an error for a missing node, wrong workflow, or request failure.
    pub async fn node(&self, workflow_id: i64, node_id: i64) -> Result<Node, BackendError> {
        self.call(
            "node.get",
            json!({"workflow_id": workflow_id, "node_id": node_id}),
        )
        .await
    }

    /// Replaces a node's saved prompt without changing its identity or links.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid input, server errors, or transport failures.
    pub async fn update_node(
        &self,
        workflow_id: i64,
        node_id: i64,
        prompt: &str,
    ) -> Result<Node, BackendError> {
        self.call(
            "node.update",
            json!({
                "workflow_id": workflow_id, "node_id": node_id, "prompt": prompt
            }),
        )
        .await
    }

    /// Explicitly selects an existing native jj change for a saved node.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid membership/selection, jj failures, or request failures.
    pub async fn associate_node(
        &self,
        workflow_id: i64,
        node_id: i64,
        change_id: &str,
    ) -> Result<NodeChange, BackendError> {
        self.call(
            "node.associate",
            json!({
                "workflow_id": workflow_id, "node_id": node_id, "change_id": change_id
            }),
        )
        .await
    }

    /// Rebases a node and descendants, requiring explicit consent to the undo boundary.
    ///
    /// # Errors
    ///
    /// Returns an error for unsafe state or transport/native/storage failures.
    /// Inspect operations after uncertainty; never automatically retry.
    pub async fn reparent_node(
        &self,
        workflow_id: i64,
        node_id: i64,
        parent: &NodeParent,
        acknowledgement: Option<UndoBoundaryAcknowledgement>,
    ) -> Result<DependencyOperation, BackendError> {
        self.call(
            "node.reparent",
            json!({
                "workflow_id": workflow_id, "node_id": node_id,
                "parent": parent, "acknowledgement": acknowledgement,
            }),
        )
        .await
    }

    /// Abandons a native node and deregisters its saved workspace while preserving all files.
    ///
    /// # Errors
    ///
    /// Returns an error for missing consent, unsafe state, or request/native/storage
    /// failure. Inspect the operation after uncertainty; never blindly retry.
    pub async fn delete_node(
        &self,
        workflow_id: i64,
        node_id: i64,
        acknowledgement: Option<UndoBoundaryAcknowledgement>,
    ) -> Result<DependencyOperation, BackendError> {
        self.call(
            "node.delete",
            json!({
                "workflow_id": workflow_id, "node_id": node_id, "acknowledgement": acknowledgement,
            }),
        )
        .await
    }

    /// Lists durable dependency edit inputs and outcomes, including unresolved reservations.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid workflow or request/storage failure.
    pub async fn dependency_operations(
        &self,
        workflow_id: i64,
    ) -> Result<Vec<DependencyOperation>, BackendError> {
        self.call("node.operations", json!({"workflow_id": workflow_id}))
            .await
    }

    /// Inspects whether an unresolved dependency edit can be resolved without rerunning jj.
    ///
    /// # Errors
    ///
    /// Returns an error for missing records or request/storage failures.
    pub async fn inspect_dependency_recovery(
        &self,
        operation_id: i64,
    ) -> Result<DependencyRecovery, BackendError> {
        self.call(
            "node.operation_recovery",
            json!({"operation_id": operation_id}),
        )
        .await
    }

    /// Explicitly resolves a provable dependency-edit outcome after writers have stopped.
    ///
    /// # Errors
    ///
    /// Returns an error for missing confirmation, ambiguous state, or request/storage failures.
    pub async fn recover_dependency_operation(
        &self,
        operation_id: i64,
        confirmation: Option<RecoveryConfirmation>,
    ) -> Result<DependencyOperation, BackendError> {
        self.call(
            "node.recover_operation",
            json!({
                "operation_id": operation_id, "confirmation": confirmation,
            }),
        )
        .await
    }

    /// Creates an empty jj change on an explicit base without switching the working copy.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid membership/base, an associated or guarded node,
    /// jj/storage failures, or transport failures. Inspect native state after an
    /// uncertain outcome; do not automatically retry creation.
    pub async fn create_node_change(
        &self,
        workflow_id: i64,
        node_id: i64,
        base: &str,
    ) -> Result<NodeChange, BackendError> {
        self.call(
            "node.create_change",
            json!({"workflow_id": workflow_id, "node_id": node_id, "base": base}),
        )
        .await
    }

    /// Allocates a native workspace at an explicit absolute directory, or reports its saved allocation.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid membership/selection/destination, native or storage
    /// failures, or transport errors. Inspect workspace status after uncertain outcomes.
    pub async fn create_node_workspace(
        &self,
        workflow_id: i64,
        node_id: i64,
        directory: &Path,
    ) -> Result<NodeWorkspace, BackendError> {
        self.call(
            "node.workspace.create",
            json!({"workflow_id": workflow_id, "node_id": node_id, "directory": directory}),
        )
        .await
    }

    /// Reads saved allocation and native status without snapshotting or updating files.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid membership, inaccessible repositories, native
    /// failures, or transport errors.
    pub async fn node_workspace(
        &self,
        workflow_id: i64,
        node_id: i64,
    ) -> Result<NodeWorkspace, BackendError> {
        self.call(
            "node.workspace.status",
            json!({"workflow_id": workflow_id, "node_id": node_id}),
        )
        .await
    }

    /// Verifies checkout completion and marks a saved allocation ready without changing native state.
    ///
    /// # Errors
    ///
    /// Returns an error for missing/mismatched allocations, unfinished or stale
    /// checkouts, unsupported jj output, native/storage failures, or transport errors.
    pub async fn verify_node_workspace(
        &self,
        workflow_id: i64,
        node_id: i64,
    ) -> Result<NodeWorkspace, BackendError> {
        self.call(
            "node.workspace.verify",
            json!({"workflow_id": workflow_id, "node_id": node_id}),
        )
        .await
    }

    /// Queries current native state without snapshotting or changing the repository.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid membership, unavailable repositories/jj, or request failures.
    pub async fn refresh_node(
        &self,
        workflow_id: i64,
        node_id: i64,
    ) -> Result<NodeChange, BackendError> {
        self.call(
            "node.refresh",
            json!({"workflow_id": workflow_id, "node_id": node_id}),
        )
        .await
    }

    /// Reviews a node's current recorded commit without snapshotting its workspace.
    ///
    /// # Errors
    ///
    /// Refuses absent/ambiguous revisions, oversized output, or native/transport failures.
    pub async fn review_node(
        &self,
        workflow_id: i64,
        node_id: i64,
    ) -> Result<waypoints_core::jj::RevisionReview, BackendError> {
        self.call(
            "node.review",
            json!({"workflow_id": workflow_id, "node_id": node_id}),
        )
        .await
    }

    /// Reviews the exact snapshot saved by a run, including historical node runs.
    ///
    /// # Errors
    ///
    /// Refuses absent snapshots, unavailable commits, oversized output, or request failures.
    pub async fn review_run(
        &self,
        run_id: i64,
    ) -> Result<waypoints_core::jj::RevisionReview, BackendError> {
        self.call("run.review", json!({"run_id": run_id})).await
    }

    /// Explicitly continues an abandoned deletion by deregistering its workspace, keeping all files.
    ///
    /// # Errors
    ///
    /// Refuses absent stopped-writer confirmation, ambiguity, or native/storage/transport failures.
    pub async fn resume_node_deletion(
        &self,
        operation_id: i64,
        confirmation: Option<RecoveryConfirmation>,
    ) -> Result<waypoints_core::storage::DependencyOperation, BackendError> {
        self.call(
            "node.resume_deletion",
            json!({
                "operation_id": operation_id, "confirmation": confirmation
            }),
        )
        .await
    }

    /// Moves a prompt-only subtree without invoking jj or clearing metadata Undo.
    ///
    /// # Errors
    ///
    /// Refuses invalid topology, native/workspace/run references, or request failures.
    pub async fn move_prompt_node(
        &self,
        workflow_id: i64,
        node_id: i64,
        parent: &waypoints_core::workflow::NodeParent,
    ) -> Result<waypoints_core::storage::WorkflowGraph, BackendError> {
        self.call(
            "node.move_prompt",
            json!({
                "workflow_id": workflow_id, "node_id": node_id, "parent": parent
            }),
        )
        .await
    }

    /// Deletes a prompt-only node and reconnects children, retaining transactional metadata Undo.
    ///
    /// # Errors
    ///
    /// Refuses native/workspace/run references, unsupported graphs, or request failures.
    pub async fn delete_prompt_node(
        &self,
        workflow_id: i64,
        node_id: i64,
    ) -> Result<waypoints_core::storage::WorkflowGraph, BackendError> {
        self.call(
            "node.delete_prompt",
            json!({"workflow_id": workflow_id, "node_id": node_id}),
        )
        .await
    }

    /// Compares saved direct upstream revisions without affecting execution or agent context.
    ///
    /// # Errors
    ///
    /// Returns an error for unreadable runs or transport failures; unavailable inputs are unknown.
    pub async fn run_upstream_status(
        &self,
        run_id: i64,
    ) -> Result<waypoints_core::run::UpstreamStatus, BackendError> {
        self.call("run.inputs", json!({"run_id": run_id})).await
    }

    /// Saves a directed connection between two nodes in one workflow.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid, duplicate, or cyclic links, or request failures.
    pub async fn connect_nodes(
        &self,
        workflow_id: i64,
        source_id: i64,
        target_id: i64,
    ) -> Result<Connection, BackendError> {
        self.call(
            "connection.create",
            json!({
                "workflow_id": workflow_id, "source_id": source_id, "target_id": target_id
            }),
        )
        .await
    }

    /// Starts a backend-owned ACP run and returns before adapter initialization finishes.
    ///
    /// # Errors
    ///
    /// Returns an error for an unready/reserved workspace, invalid executable,
    /// storage failure, or transport failure. Never automatically retry a start.
    pub async fn start_run(
        &self,
        workflow_id: i64,
        node_id: i64,
        executable: &Path,
    ) -> Result<RunStatus, BackendError> {
        self.call(
            "run.start",
            json!({"workflow_id":workflow_id,"node_id":node_id,"executable":executable}),
        )
        .await
    }

    /// Starts an explicit follow-up by loading a stopped run's saved ACP session.
    ///
    /// The new run captures the follow-up without changing the node assignment.
    /// It reuses the saved executable and workspace. A load failure never creates
    /// a fresh session.
    ///
    /// # Errors
    ///
    /// Returns an error for an ineligible source, invalid prompt, unready/reserved
    /// workspace, or request failure. Never automatically retry an uncertain response.
    pub async fn continue_run(&self, run_id: i64, prompt: &str) -> Result<RunStatus, BackendError> {
        self.call("run.continue", json!({"run_id":run_id,"prompt":prompt}))
            .await
    }

    /// Inspects interrupted/recovered metadata and read-only workspace checks.
    ///
    /// A verified workspace does not prove old processes stopped. This call never
    /// releases reservations, signals saved PIDs, or records a run revision.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown/ineligible run or request failure.
    pub async fn inspect_run_recovery(&self, run_id: i64) -> Result<RunRecovery, BackendError> {
        self.call("run.recovery", json!({"run_id":run_id})).await
    }

    /// Explicitly recovers a reservation after caller attestation and fresh workspace checks.
    ///
    /// Does not execute, snapshot, or repair anything. Repeating recovery returns
    /// its original outcome without releasing a newer run's reservation.
    ///
    /// # Errors
    ///
    /// Returns an error for ineligible/live runs, failed verification, or request failure.
    pub async fn recover_run(
        &self,
        run_id: i64,
        confirmation: RecoveryConfirmation,
    ) -> Result<RunStatus, BackendError> {
        self.call(
            "run.recover",
            json!({"run_id":run_id,"confirmation":confirmation}),
        )
        .await
    }

    /// Reads a durable run outcome and any live approval.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown run or request failure.
    pub async fn run_status(&self, run_id: i64) -> Result<RunStatus, BackendError> {
        self.call("run.status", json!({"run_id":run_id})).await
    }

    /// Lists a node's runs, including those from earlier backend instances.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid membership or request failure.
    pub async fn node_runs(
        &self,
        workflow_id: i64,
        node_id: i64,
    ) -> Result<Vec<RunStatus>, BackendError> {
        self.call(
            "run.list",
            json!({"workflow_id":workflow_id,"node_id":node_id}),
        )
        .await
    }

    /// Polls bounded live output; callers must surface a reported event gap.
    ///
    /// # Errors
    ///
    /// Returns an error for expired run buffers, invalid cursors, or request failures.
    pub async fn run_events(&self, run_id: i64, after: u64) -> Result<RunEvents, BackendError> {
        self.call("run.events", json!({"run_id":run_id,"after":after}))
            .await
    }

    /// Requests cancellation without waiting for process cleanup.
    ///
    /// # Errors
    ///
    /// Returns an error for a run not owned by this backend or request failure.
    pub async fn cancel_run(&self, run_id: i64) -> Result<RunStatus, BackendError> {
        self.call("run.cancel", json!({"run_id":run_id})).await
    }

    /// Answers exactly one pending approval using an advertised single-use option.
    ///
    /// # Errors
    ///
    /// Returns an error for stale/unknown permissions or options, or request failures.
    pub async fn respond_to_permission(
        &self,
        run_id: i64,
        permission_id: u64,
        option_id: &str,
    ) -> Result<RunStatus, BackendError> {
        self.call(
            "run.permission",
            json!({"run_id":run_id,"permission_id":permission_id,"option_id":option_id}),
        )
        .await
    }

    /// Requests orderly backend shutdown without deleting saved workflows.
    ///
    /// # Errors
    ///
    /// Returns an error if the shutdown request cannot be delivered or acknowledged.
    pub async fn stop(&self) -> Result<(), BackendError> {
        self.call("backend.shutdown", json!({})).await
    }

    async fn call<T: DeserializeOwned>(
        &self,
        method: &str,
        params: Value,
    ) -> Result<T, BackendError> {
        Ok(timeout(crate::protocol::request_timeout(method), async {
            let stream = UnixStream::connect(&self.config.socket)
                .await
                .context("connecting to backend")?;
            exchange(stream, method, params).await
        })
        .await
        .context("backend request timed out")??)
    }
}

fn unavailable(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    )
}

async fn exchange<T: DeserializeOwned>(
    stream: UnixStream,
    method: &str,
    params: Value,
) -> anyhow::Result<T> {
    timeout(crate::protocol::request_timeout(method), async {
        let mut stream = BufReader::new(stream);
        let mut request = serde_json::to_vec(&json!({
            "jsonrpc": "2.0", "id": 1, "method": method, "params": params
        }))?;
        request.push(b'\n');
        anyhow::ensure!(request.len() <= MAX_FRAME, "request exceeds 1 MiB");
        stream
            .get_mut()
            .write_all(&request)
            .await
            .context("sending backend request")?;
        let response: Value = serde_json::from_slice(&read_frame(&mut stream).await?)
            .context("decoding backend response")?;
        anyhow::ensure!(
            response["jsonrpc"] == "2.0" && response["id"] == 1,
            "invalid backend response"
        );
        if let Some(error) = response.get("error") {
            anyhow::bail!("backend error {}: {}", error["code"], error["message"]);
        }
        serde_json::from_value(
            response
                .get("result")
                .context("backend response has no result")?
                .clone(),
        )
        .context("decoding backend result")
    })
    .await
    .context("backend request timed out")?
}
