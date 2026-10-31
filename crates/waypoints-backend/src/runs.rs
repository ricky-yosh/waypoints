use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::{Instant, timeout};
use waypoints_core::jj::Revision;
use waypoints_core::run::{InterruptionPhase, RecoveryConfirmation, Run, RunState, SnapshotState};
use waypoints_core::storage::Storage;

use crate::protocol::read_frame;

// Live output is bounded independently of the agent's session history.
const EVENT_BYTES: usize = 256 * 1024;
const EVENT_COUNT: usize = 128;
const RETAINED_RUNS: usize = 32;
const ACTIVE_RUNS: usize = 4;
const ACP_FRAME_BYTES: usize = 64 * 1024;
const SETUP_TIMEOUT: Duration = Duration::from_secs(30);
const PERMISSION_TIMEOUT: Duration = Duration::from_secs(60);
const RUN_TIMEOUT: Duration = Duration::from_secs(60 * 60);
const CANCEL_GRACE: Duration = Duration::from_millis(500);
// Allow the core's 60-second snapshot deadline plus metadata/dispatch overhead.
const SNAPSHOT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(75);

/// A pending approval, available from status even if its event has expired.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Permission {
    /// Run-local identifier; replies to old approvals are rejected.
    pub id: u64,
    /// Agent-provided tool description, not evidence the action completed.
    pub tool_call: Value,
    /// Only one-time allow/deny choices, never blanket authorization.
    pub options: Vec<PermissionOption>,
}

/// A single-use agent permission option.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PermissionOption {
    /// Opaque agent-provided option identifier.
    pub option_id: String,
    /// Agent-provided display label.
    pub name: String,
    /// ACP kind: allow_once or reject_once.
    pub kind: String,
}

/// Durable run metadata and any live approval.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RunStatus {
    /// Captured assignment and last persisted state.
    pub run: Run,
    /// Pending live request; absent after restart.
    pub permission: Option<Permission>,
}

/// A sequenced ACP update, which may contain proposed rather than completed edits.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RunEvent {
    /// Monotonic, run-local cursor.
    pub sequence: u64,
    /// Loading-time replay versus new session activity; never execute replayed tools.
    #[serde(default)]
    pub origin: RunEventOrigin,
    /// Original ACP session update.
    pub update: Value,
}

/// Whether an update arrived during session loading or ordinary activity.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RunEventOrigin {
    /// Received before session/load completed; may describe historical tool calls.
    Replay,
    /// Received outside session/load; not proof that a proposed edit completed.
    #[default]
    Live,
}

/// A bounded page of live events; not durable conversation history.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RunEvents {
    /// Current status, including any pending approval.
    pub status: RunStatus,
    /// Events newer than the requested cursor.
    pub events: Vec<RunEvent>,
    /// Cursor to use for the next poll.
    pub next: u64,
    /// Some requested events expired; callers must not silently omit the gap.
    pub gap: bool,
}

struct Live {
    status: RunStatus,
    events: VecDeque<(RunEvent, usize)>,
    bytes: usize,
    sequence: u64,
    cancel: watch::Sender<bool>,
    replies: mpsc::Sender<(u64, String)>,
    task: JoinHandle<()>,
}

pub(crate) enum Update {
    Event(Value, RunEventOrigin),
    Process(u32),
    Session(String),
    State(RunState),
    Permission(Permission),
    Finished(RunState),
    Snapshotted(Result<Revision, String>),
}

pub(crate) struct Runs {
    live: BTreeMap<i64, Live>,
    tx: mpsc::Sender<(i64, Update)>,
    pub(crate) rx: mpsc::Receiver<(i64, Update)>,
}

impl Runs {
    pub(crate) fn new() -> Self {
        let (tx, rx) = mpsc::channel(32);
        Self {
            live: BTreeMap::new(),
            tx,
            rx,
        }
    }

    pub(crate) fn ensure_capacity(&mut self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.active_count() < ACTIVE_RUNS,
            "at most {ACTIVE_RUNS} active runs are supported"
        );
        if self.live.len() >= RETAINED_RUNS {
            let id = self
                .live
                .iter()
                .find(|(_, live)| live.task.is_finished() && terminal(&live.status.run.state))
                .map(|(id, _)| *id)
                .context("live run capacity reached")?;
            self.live.remove(&id);
        }
        Ok(())
    }

    pub(crate) fn start(&mut self, run: Run) -> RunStatus {
        let (cancel, cancelled) = watch::channel(false);
        let (replies, responses) = mpsc::channel(1);
        let status = RunStatus {
            run: run.clone(),
            permission: None,
        };
        let task = tokio::spawn(worker(run, self.tx.clone(), cancelled, responses));
        self.live.insert(
            status.run.id,
            Live {
                status: status.clone(),
                events: VecDeque::new(),
                bytes: 0,
                sequence: 0,
                cancel,
                replies,
                task,
            },
        );
        status
    }

    pub(crate) fn status(&self, run: Run) -> RunStatus {
        self.live
            .get(&run.id)
            .map(|live| live.status.clone())
            .unwrap_or(RunStatus {
                run,
                permission: None,
            })
    }

    pub(crate) fn events(&self, id: i64, after: u64) -> anyhow::Result<RunEvents> {
        let live = self.live.get(&id).context(
            "live events unavailable after restart or eviction; inspect saved run status",
        )?;
        anyhow::ensure!(after <= live.sequence, "event cursor is ahead of this run");
        let first = live
            .events
            .front()
            .map(|(event, _)| event.sequence)
            .unwrap_or(live.sequence + 1);
        Ok(RunEvents {
            status: live.status.clone(),
            events: live
                .events
                .iter()
                .filter(|(event, _)| event.sequence > after)
                .map(|(event, _)| event.clone())
                .collect(),
            next: live.sequence,
            gap: after + 1 < first,
        })
    }

    pub(crate) async fn cancel(
        &mut self,
        id: i64,
        storage: &mut Storage,
    ) -> anyhow::Result<RunStatus> {
        let live = self
            .live
            .get_mut(&id)
            .context("run is not owned by this backend; inspect saved status")?;
        if !live.task.is_finished()
            && !terminal(&live.status.run.state)
            && !matches!(live.status.run.state, RunState::Finalizing)
        {
            live.cancel.send_replace(true);
            live.status.run.state = RunState::Cancelling;
            live.status.permission = None;
            storage.save_run(&live.status.run).await?;
        }
        Ok(live.status.clone())
    }

    pub(crate) async fn recover(
        &mut self,
        id: i64,
        confirmation: RecoveryConfirmation,
        storage: &mut Storage,
    ) -> anyhow::Result<RunStatus> {
        if let Some(live) = self.live.get(&id) {
            anyhow::ensure!(
                live.task.is_finished(),
                "run still has an owned task; wait for it to stop before recovery"
            );
        }
        let run = storage.recover_run(id, confirmation).await?;
        // A cleanup-failed run may still have a retained live event buffer.
        // Do not let that cached interruption override the durable recovery.
        if let Some(live) = self.live.get_mut(&id) {
            live.status.run = run.clone();
            live.status.permission = None;
        }
        Ok(self.status(run))
    }

    pub(crate) async fn respond(
        &mut self,
        id: i64,
        permission_id: u64,
        option_id: String,
        storage: &mut Storage,
    ) -> anyhow::Result<RunStatus> {
        let live = self
            .live
            .get_mut(&id)
            .context("run is not owned by this backend")?;
        let permission = live
            .status
            .permission
            .as_ref()
            .context("run has no pending approval")?;
        anyhow::ensure!(
            permission.id == permission_id,
            "stale permission identifier"
        );
        anyhow::ensure!(
            permission
                .options
                .iter()
                .any(|option| option.option_id == option_id),
            "unknown or non-single-use permission option"
        );
        live.replies
            .try_send((permission_id, option_id))
            .context("permission reply queue unavailable")?;
        live.status.permission = None;
        live.status.run.state = RunState::Running;
        storage.save_run(&live.status.run).await?;
        Ok(live.status.clone())
    }

    pub(crate) async fn apply(
        &mut self,
        id: i64,
        update: Update,
        storage: &mut Storage,
    ) -> anyhow::Result<()> {
        let live = self.live.get_mut(&id).context("update for unknown run")?;
        match update {
            Update::Event(update, origin) => {
                let bytes = serde_json::to_vec(&update)?.len() + 128;
                live.sequence += 1;
                live.bytes += bytes;
                live.events.push_back((
                    RunEvent {
                        sequence: live.sequence,
                        origin,
                        update,
                    },
                    bytes,
                ));
                while live.bytes > EVENT_BYTES || live.events.len() > EVENT_COUNT {
                    if let Some((_, bytes)) = live.events.pop_front() {
                        live.bytes -= bytes;
                    }
                }
                return Ok(());
            }
            Update::Process(pid) => live.status.run.process_id = Some(pid),
            Update::Session(session) => live.status.run.session_id = Some(session),
            Update::State(state) => {
                if !matches!(live.status.run.state, RunState::Cancelling) {
                    live.status.run.state = state;
                }
            }
            Update::Permission(permission) => {
                // Cancellation wins over an already queued permission request.
                if !*live.cancel.borrow() {
                    live.status.permission = Some(permission);
                    live.status.run.state = RunState::AwaitingPermission;
                }
            }
            Update::Finished(state) => {
                live.status.permission = None;
                if matches!(state, RunState::Completed) {
                    // Cleanup is confirmed. Persist ownership across native work;
                    // clients must never observe completed before finalization.
                    live.status.run.state = RunState::Finalizing;
                    storage.save_run(&live.status.run).await?;
                    let operation = storage.prepare_run_snapshot(id).await;
                    match operation {
                        Ok(operation) => {
                            (&mut live.task)
                                .await
                                .context("joining completed agent task")?;
                            let tx = self.tx.clone();
                            live.task = tokio::spawn(async move {
                                let result = operation
                                    .record()
                                    .await
                                    .map_err(|error| format!("{error:#}"));
                                let _ = tx.send((id, Update::Snapshotted(result))).await;
                            });
                            return Ok(());
                        }
                        Err(error) => {
                            live.status.run.snapshot = SnapshotState::Failed {
                                detail: format!("{error:#}"),
                            };
                        }
                    }
                }
                live.status.run.state = state;
            }
            Update::Snapshotted(result) => {
                anyhow::ensure!(
                    matches!(live.status.run.state, RunState::Finalizing),
                    "snapshot result for a non-finalizing run"
                );
                live.status.run.snapshot = match result {
                    Ok(revision) => SnapshotState::Saved { revision },
                    Err(detail) => SnapshotState::Failed { detail },
                };
                live.status.run.state = RunState::Completed;
            }
        }
        storage.save_run(&live.status.run).await?;
        Ok(())
    }

    fn active_count(&self) -> usize {
        self.live
            .values()
            .filter(|live| !live.task.is_finished() || !terminal(&live.status.run.state))
            .count()
    }

    pub(crate) async fn shutdown(&mut self, storage: &mut Storage) -> anyhow::Result<()> {
        for live in self.live.values() {
            live.cancel.send_replace(true);
        }
        // Workers send their final outcome only after stopping and reaping their process.
        while self
            .live
            .values()
            .any(|live| !terminal(&live.status.run.state))
        {
            // Snapshotting is non-cancellable once cleanup has finished. Keep
            // locks and reservations until its bounded native operation settles.
            let wait = if self
                .live
                .values()
                .any(|live| matches!(live.status.run.state, RunState::Finalizing))
            {
                SNAPSHOT_SHUTDOWN_TIMEOUT
            } else {
                Duration::from_secs(5)
            };
            let (id, update) = timeout(wait, self.rx.recv())
                .await
                .context("agent shutdown timed out; reservations retained")?
                .context("agent update channel closed")?;
            self.apply(id, update, storage).await?;
        }
        for live in self.live.values_mut() {
            (&mut live.task).await.context("joining agent task")?;
        }
        Ok(())
    }
}

fn terminal(state: &RunState) -> bool {
    matches!(
        state,
        RunState::Completed
            | RunState::Cancelled
            | RunState::Failed { .. }
            | RunState::Interrupted { .. }
            | RunState::Recovered { .. }
    )
}

struct Agent {
    child: Child,
    input: ChildStdin,
    frames: mpsc::Receiver<anyhow::Result<Value>>,
    reader: JoinHandle<()>,
}

impl Agent {
    fn spawn(executable: &PathBuf, directory: &PathBuf) -> anyhow::Result<Self> {
        let mut child = Command::new(executable)
            .current_dir(directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // No inherited terminal or unbounded stderr capture.
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .process_group(0)
            .spawn()
            .context("starting ACP executable")?;
        let input = child.stdin.take().context("ACP stdin unavailable")?;
        let output = child.stdout.take().context("ACP stdout unavailable")?;
        let (tx, frames) = mpsc::channel(8);
        let reader = tokio::spawn(read_agent(output, tx));
        Ok(Self {
            child,
            input,
            frames,
            reader,
        })
    }

    async fn send(&mut self, message: Value) -> anyhow::Result<()> {
        let mut bytes = serde_json::to_vec(&message)?;
        anyhow::ensure!(
            bytes.len() < crate::protocol::MAX_FRAME,
            "ACP request exceeds 1 MiB"
        );
        bytes.push(b'\n');
        timeout(Duration::from_secs(2), self.input.write_all(&bytes))
            .await
            .context("ACP write timed out")??;
        Ok(())
    }

    async fn stop(&mut self) -> anyhow::Result<()> {
        self.reader.abort();
        let pid = self.child.id();
        if let Some(pid) = pid {
            // The adapter and its descendants have their own process group.
            let status = timeout(
                Duration::from_secs(2),
                Command::new("/bin/kill")
                    .args(["-KILL", "--", &format!("-{pid}")])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status(),
            )
            .await
            .context("stopping ACP process group timed out")??;
            anyhow::ensure!(
                status.success() || self.child.try_wait()?.is_some(),
                "could not stop ACP process group"
            );
        }
        timeout(Duration::from_secs(2), self.child.wait())
            .await
            .context("reaping ACP process timed out")??;
        Ok(())
    }
}

async fn read_agent(output: impl AsyncRead + Unpin, tx: mpsc::Sender<anyhow::Result<Value>>) {
    let mut output = BufReader::new(output);
    loop {
        let message = async {
            let bytes = read_frame(&mut output).await?;
            anyhow::ensure!(bytes.len() <= ACP_FRAME_BYTES, "ACP event exceeds 64 KiB");
            serde_json::from_slice(&bytes).context("invalid ACP JSON")
        }
        .await;
        let failed = message.is_err();
        if tx.send(message).await.is_err() || failed {
            break;
        }
    }
}

async fn worker(
    run: Run,
    tx: mpsc::Sender<(i64, Update)>,
    mut cancel: watch::Receiver<bool>,
    mut replies: mpsc::Receiver<(u64, String)>,
) {
    let mut agent = match Agent::spawn(&run.executable, &run.directory) {
        Ok(agent) => agent,
        Err(error) => {
            let _ = tx
                .send((
                    run.id,
                    Update::Finished(RunState::Failed {
                        detail: format!("{error:#}"),
                    }),
                ))
                .await;
            return;
        }
    };
    let result = async {
        let pid = agent
            .child
            .id()
            .context("ACP process exited before initialization")?;
        tx.send((run.id, Update::Process(pid))).await?;
        conversation(&run, &mut agent, &tx, &mut cancel, &mut replies).await
    }
    .await;
    let state = match agent.stop().await {
        Err(error) => RunState::Interrupted {
            detail: format!(
                "process cleanup unconfirmed: {error:#}; inspect workspace and native processes"
            ),
            phase: InterruptionPhase::Execution,
        },
        Ok(()) => match result {
            Ok(state) => state,
            Err(error) => RunState::Failed {
                detail: format!("{error:#}"),
            },
        },
    };
    let _ = tx.send((run.id, Update::Finished(state))).await;
}

async fn conversation(
    run: &Run,
    agent: &mut Agent,
    tx: &mpsc::Sender<(i64, Update)>,
    cancel: &mut watch::Receiver<bool>,
    replies: &mut mpsc::Receiver<(u64, String)>,
) -> anyhow::Result<RunState> {
    agent
        .send(request(
            1,
            "initialize",
            json!({
                "protocolVersion": 1, "clientCapabilities": {},
                "clientInfo": {"name": "waypoints", "version": env!("CARGO_PKG_VERSION")}
            }),
        ))
        .await?;
    let mut phase = 1;
    // Load replays updates before its response; validate them against the saved ID.
    let mut session = run.continued_from.and(run.session_id.clone());
    let mut pending: Option<(u64, Value, Vec<PermissionOption>)> = None;
    let mut permission_id = 0;
    let mut deadline = Instant::now() + SETUP_TIMEOUT;
    let run_deadline = Instant::now() + RUN_TIMEOUT;
    let mut cancelling = false;
    loop {
        tokio::select! {
            biased;
            _ = cancel.changed(), if !cancelling => {
                if let Some(session) = &session {
                    let _ = agent.send(json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":session}})).await;
                }
                cancelling = true;
                pending = None;
                deadline = Instant::now() + CANCEL_GRACE;
            }
            _ = tokio::time::sleep_until(deadline.min(run_deadline)) => {
                if cancelling { return Ok(RunState::Cancelled); }
                if pending.is_some() {
                    let _ = agent.send(json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":session}})).await;
                    return Ok(RunState::Cancelled);
                }
                anyhow::bail!("ACP setup or run deadline exceeded");
            }
            Some((id, option)) = replies.recv(), if pending.is_some() && !cancelling => {
                let (expected, rpc_id, options) = pending.take().context("approval disappeared")?;
                anyhow::ensure!(id == expected && options.iter().any(|item| item.option_id == option), "invalid permission reply");
                agent.send(json!({"jsonrpc":"2.0","id":rpc_id,"result":{"outcome":{"outcome":"selected","optionId":option}}})).await?;
                deadline = run_deadline;
            }
            frame = agent.frames.recv() => {
                let message = frame.context("ACP output closed")??;
                anyhow::ensure!(message["jsonrpc"] == "2.0", "invalid ACP envelope");
                if let Some(method) = message.get("method").and_then(Value::as_str) {
                    if method == "session/update" {
                        anyhow::ensure!(message["params"]["sessionId"].as_str() == session.as_deref(), "ACP update for wrong session");
                        if !cancelling {
                            let origin = if phase == 2 && run.continued_from.is_some() {
                                RunEventOrigin::Replay
                            } else {
                                RunEventOrigin::Live
                            };
                            tx.send((run.id, Update::Event(message["params"]["update"].clone(), origin))).await?;
                        }
                    } else if method == "session/request_permission" && !cancelling {
                        anyhow::ensure!(phase == 4 && pending.is_none(), "unexpected or overlapping ACP permission");
                        anyhow::ensure!(message["params"]["sessionId"].as_str() == session.as_deref(), "permission for wrong session");
                        let rpc_id = message.get("id").filter(|id| id.is_string() || id.is_number()).context("permission has no request identifier")?.clone();
                        let mut options = Vec::new();
                        for option in message["params"]["options"].as_array().context("invalid permission options")? {
                            let kind = option["kind"].as_str().context("invalid permission kind")?;
                            if matches!(kind, "allow_once" | "reject_once") {
                                options.push(PermissionOption {
                                    option_id: option["optionId"].as_str().context("invalid option identifier")?.into(),
                                    name: option["name"].as_str().context("invalid option name")?.into(), kind: kind.into(),
                                });
                            }
                        }
                        permission_id += 1;
                        pending = Some((permission_id, rpc_id, options.clone()));
                        tx.send((run.id, Update::Permission(Permission {
                            id: permission_id, tool_call: message["params"]["toolCall"].clone(), options,
                        }))).await?;
                        deadline = Instant::now() + PERMISSION_TIMEOUT;
                    } else if let Some(id) = message.get("id") {
                        // No client filesystem, terminal, or auth callbacks were advertised.
                        agent.send(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"unsupported client callback"}})).await?;
                    }
                    continue;
                }
                anyhow::ensure!(message["id"] == phase, "unexpected ACP response identifier");
                if cancelling { return Ok(RunState::Cancelled); }
                anyhow::ensure!(message.get("error").is_none(), "ACP {} failed: {}", if phase == 2 && run.continued_from.is_some() { "session/load (no fresh-session fallback)" } else { "request" }, message["error"]);
                let result = message.get("result").context("ACP response has no result")?;
                anyhow::ensure!(result.is_object(), "ACP response result must be an object");
                match phase {
                    1 => {
                        anyhow::ensure!(result["protocolVersion"] == 1, "unsupported ACP protocol version");
                        if run.continued_from.is_some() {
                            anyhow::ensure!(result["agentCapabilities"]["loadSession"] == true, "agent does not advertise loadSession; continuation will not start a fresh session");
                            anyhow::ensure!(session.is_some(), "continuation has no saved session identifier");
                            agent.send(request(2,"session/load",json!({"sessionId":session,"cwd":run.directory,"mcpServers":[]}))).await?;
                        } else {
                            agent.send(request(2,"session/new",json!({"cwd":run.directory,"mcpServers":[]}))).await?;
                        }
                    }
                    2 => {
                        if run.continued_from.is_none() {
                            let id = result["sessionId"].as_str().filter(|id| !id.is_empty()).context("ACP session has no identifier")?.to_owned();
                            session = Some(id.clone());
                            tx.send((run.id, Update::Session(id))).await?;
                        } else if let Some(id) = result.get("sessionId") {
                            anyhow::ensure!(id.as_str() == session.as_deref(), "ACP load returned a different session identifier");
                        }
                        // Initial modes are optional on load. A successful set_mode
                        // is still required before sending any new prompt.
                        if run.continued_from.is_none() || result.get("modes").is_some() {
                            let modes = result["modes"]["availableModes"].as_array().context("agent does not advertise permission modes")?;
                            anyhow::ensure!(modes.iter().any(|mode| mode["id"] == "workspace-write"), "agent does not support explicit workspace-write mode");
                        }
                        agent.send(request(3,"session/set_mode",json!({"sessionId":session,"modeId":"workspace-write"}))).await?;
                    }
                    3 => {
                        agent.send(request(4,"session/prompt",json!({"sessionId":session,"prompt":[{"type":"text","text":run.prompt}]}))).await?;
                        tx.send((run.id, Update::State(RunState::Running))).await?;
                        deadline = run_deadline;
                    }
                    4 => return match result["stopReason"].as_str() {
                        Some("end_turn") => Ok(RunState::Completed),
                        Some("cancelled") => Ok(RunState::Cancelled),
                        other => anyhow::bail!("agent stopped without completing the turn: {other:?}"),
                    },
                    _ => unreachable!("ACP handshake has four phases"),
                }
                phase += 1;
            }
        }
    }
}

fn request(id: i32, method: &str, params: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
}
