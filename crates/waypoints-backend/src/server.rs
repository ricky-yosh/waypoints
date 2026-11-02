use std::fs::{self, File, OpenOptions};
use std::future::Future;
use std::io;
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::pin::Pin;

use anyhow::Context;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::task::JoinSet;
use tokio::time::timeout;
use waypoints_core::storage::Storage;

use crate::config::sidecar;
use crate::protocol::{
    AdoptRootsParams, AssociateParams, ConnectParams, ContinueRunParams, CreateChangeParams,
    CreateNodeParams, CreateParams, CreateWorkspaceParams, DeleteNodeParams,
    DependencyOperationParams, HistoryParams, MAX_FRAME, NodeParams, NodeRevisionParams,
    NodeRevisionsParams, PROTOCOL_VERSION, PermissionParams, ProjectParams,
    RecoverDependencyParams, RecoverRunParams, RenameWorkflowParams, ReparentParams,
    RestoreDescendantsParams, RunEventsParams, RunParams, StartRunParams, TIMEOUT,
    UpdateNodeParams, WorkflowBaseParams, WorkflowParams, read_frame,
};
use crate::runs::{PersistedRun, Runs};
use crate::{BackendError, BackendInfo, Config};

/// Runs the local server until a shutdown request or Ctrl+C.
///
/// Storage work is serialized; live run controls bypass that lane. Each connection
/// carries one newline-delimited JSON-RPC request, bounded to 1 MiB.
/// Ownership locks are acquired before opening storage;
/// stale sockets are removed only while holding both locks.
///
/// # Errors
///
/// Returns an error if ownership cannot be acquired, storage cannot be opened,
/// or the listener fails. Individual malformed client requests do not stop the server.
pub async fn serve(config: &Config) -> Result<(), BackendError> {
    Ok(run(config).await?)
}

async fn run(config: &Config) -> anyhow::Result<()> {
    let _database_lock = lock(&sidecar(&config.database, ".lock"))?;
    let _socket_lock = lock(&sidecar(&config.socket, ".lock"))?;
    match fs::symlink_metadata(&config.socket) {
        Ok(metadata) => {
            anyhow::ensure!(
                metadata.file_type().is_socket(),
                "socket path exists and is not a socket"
            );
            match UnixStream::connect(&config.socket).await {
                Ok(_) => anyhow::bail!("socket already has a live listener"),
                Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {
                    fs::remove_file(&config.socket).context("removing stale socket")?;
                }
                Err(error) => return Err(error).context("checking existing socket"),
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("checking socket path"),
    }
    let mut storage = Storage::open(&config.database).await?;
    storage.interrupt_runs().await?;
    let mut runs = Runs::new();
    let listener = UnixListener::bind(&config.socket).context("binding backend socket")?;
    let _socket_cleanup = SocketCleanup(config.socket.clone());
    fs::set_permissions(&config.socket, fs::Permissions::from_mode(0o600))
        .context("restricting socket permissions")?;
    let info = BackendInfo {
        pid: std::process::id(),
        database: config.database.clone(),
        protocol_version: PROTOCOL_VERSION,
    };
    let mut storage = Some(storage);
    let mut active: Option<Job> = None;
    let mut connections = JoinSet::new();
    let serving = async {
        loop {
            if active.is_none()
                && let Some(write) = runs.next_write()
            {
                let mut owned = storage.take().expect("idle scheduler owns storage");
                active = Some(Box::pin(async move {
                    let result = write.persist(&mut owned).await;
                    (owned, Completion::Run(result))
                }));
            }
            tokio::select! {
                completed = wait_job(&mut active) => {
                    active = None;
                    storage = Some(completed.0);
                    match completed.1 {
                        Completion::Run(result) => runs.persisted(result?)?,
                        Completion::Request { stream, request, response } => {
                            let response = finish_request(&request, response, &mut runs)?;
                            respond(&mut connections, stream, response);
                        }
                    }
                }
                Some((id, update)) = runs.rx.recv() => {
                    runs.apply(id, update)?;
                }
                incoming = listener.accept() => {
                    let stream = incoming.context("accepting backend connection")?.0;
                    // Admission and framing have no storage access. Slow readers
                    // and writers consume bounded slots, not the dispatch lane.
                    if connections.len() < CONNECTIONS {
                        connections.spawn(read_request(stream));
                    }
                }
                Some(completed) = connections.join_next() => {
                    let Ok(Some((stream, request))) = completed else { continue };
                    let request = match request {
                        Ok(request) => request,
                        Err(_) => {
                            respond(&mut connections, stream, Some(error(Value::Null, -32700, "parse error")));
                            continue;
                        }
                    };
                    if let Some((response, stop)) = control(&request, &info, &mut runs) {
                        respond(&mut connections, stream, response);
                        if stop { break; }
                    } else if active.is_some() {
                        respond(&mut connections, stream, busy(&request));
                    } else if let Err(error) = prepare_request(&request, &mut runs) {
                        respond(&mut connections, stream, reply(&request, Err(error)));
                    } else {
                        let mut owned = storage.take().expect("idle scheduler owns storage");
                        active = Some(Box::pin(async move {
                            let response = dispatch(&request, &mut owned).await;
                            (owned, Completion::Request { stream, request, response })
                        }));
                    }
                }
                signal = tokio::signal::ctrl_c() => {
                    signal.context("waiting for shutdown signal")?;
                    break;
                }
            }
        }
        Ok::<_, anyhow::Error>(())
    }
    .await;
    // A shutdown signal or failed scheduler must never drop an uncertain native
    // future. Settle it and retain both ownership locks through run cleanup.
    runs.cancel_all();
    let settlement = if let Some(job) = active.take() {
        let (owned, completed) = job.await;
        storage = Some(owned);
        match completed {
            Completion::Run(result) => result.and_then(|result| runs.persisted(result)),
            Completion::Request {
                stream,
                request,
                response,
            } => finish_request(&request, response, &mut runs).map(|response| {
                respond(&mut connections, stream, response);
            }),
        }
    } else {
        Ok(())
    };
    let cleanup = runs
        .shutdown(storage.as_mut().expect("settled scheduler owns storage"))
        .await;
    // Framing/response tasks have their own deadlines and no native ownership.
    while connections.join_next().await.is_some() {}
    serving?;
    settlement?;
    cleanup?;
    Ok(())
}

fn lock(path: &Path) -> anyhow::Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("opening ownership lock {}", path.display()))?;
    file.try_lock()
        .with_context(|| format!("backend ownership unavailable at {}", path.display()))?;
    // Never unlink lock files: a new inode would allow two processes to own the same path.
    Ok(file)
}

struct SocketCleanup(PathBuf);

impl Drop for SocketCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

// Limits simultaneous incomplete requests and response writers, not just frames.
const CONNECTIONS: usize = 64;
type Job = Pin<Box<dyn Future<Output = (Storage, Completion)>>>;

enum Completion {
    Request {
        stream: UnixStream,
        request: Value,
        response: Result<Value, crate::RpcError>,
    },
    Run(anyhow::Result<PersistedRun>),
}

async fn wait_job(active: &mut Option<Job>) -> (Storage, Completion) {
    match active {
        Some(job) => job.await,
        None => std::future::pending().await,
    }
}

type Incoming = Option<(UnixStream, serde_json::Result<Value>)>;

async fn read_request(stream: UnixStream) -> Incoming {
    let mut stream = BufReader::new(stream);
    let Ok(Ok(bytes)) = timeout(TIMEOUT, read_frame(&mut stream)).await else {
        return None;
    };
    Some((stream.into_inner(), serde_json::from_slice(&bytes)))
}

fn respond(connections: &mut JoinSet<Incoming>, stream: UnixStream, response: Option<Value>) {
    connections.spawn(async move {
        let _ = write_response(stream, response).await;
        None
    });
}

async fn write_response(mut stream: UnixStream, response: Option<Value>) -> anyhow::Result<()> {
    if let Some(response) = response {
        let mut bytes = serde_json::to_vec(&response)?;
        bytes.push(b'\n');
        if bytes.len() > MAX_FRAME {
            bytes = serde_json::to_vec(&error(
                response["id"].clone(),
                -32000,
                "result exceeds 1 MiB",
            ))?;
            bytes.push(b'\n');
        }
        timeout(TIMEOUT, stream.write_all(&bytes))
            .await
            .context("client response timed out")??;
    }
    Ok(())
}

fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn reply(request: &Value, result: Result<Value, crate::RpcError>) -> Option<Value> {
    let id = request.get("id")?;
    Some(match result {
        Ok(result) => json!({"jsonrpc":"2.0", "id":id, "result":result}),
        Err(error) => json!({"jsonrpc":"2.0", "id":id, "error":error}),
    })
}

fn busy(request: &Value) -> Option<Value> {
    reply(request, Err(crate::RpcError::busy()))
}

/// Only explicitly live controls bypass storage ownership. Historical lookups
/// still use the ordinary lane; no second database connection is opened here.
fn control(request: &Value, info: &BackendInfo, runs: &mut Runs) -> Option<(Option<Value>, bool)> {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let valid_id = id.is_null() || id.is_string() || id.is_number();
    let Some(method) = request
        .get("method")
        .and_then(Value::as_str)
        .filter(|_| request["jsonrpc"] == "2.0" && valid_id)
    else {
        return Some((Some(error(Value::Null, -32600, "invalid request")), false));
    };
    let params = request.get("params").cloned().unwrap_or(json!({}));
    let mut stop = false;
    let result = match method {
        "backend.info" | "backend.shutdown" => {
            if params != json!({}) {
                Err(crate::RpcError::new(
                    -32602,
                    "expected empty parameters".to_owned(),
                ))
            } else {
                stop = method == "backend.shutdown";
                Ok(if stop { Value::Null } else { json!(info) })
            }
        }
        "run.status" => match parameters::<RunParams>(params) {
            Ok(params) => Ok(json!(runs.live_status(params.run_id)?)),
            Err(error) => Err(error),
        },
        "run.events" => match parameters::<RunEventsParams>(params) {
            Ok(params) => runs
                .events(params.run_id, params.after)
                .map(|events| json!(events))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "run.cancel" => match parameters::<RunParams>(params) {
            Ok(params) => runs
                .cancel(params.run_id)
                .map(|status| json!(status))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "run.permission" => match parameters::<PermissionParams>(params) {
            Ok(params) => runs
                .respond(params.run_id, params.permission_id, params.option_id)
                .map(|status| json!(status))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        _ => return None,
    };
    Some((reply(request, result), stop))
}

fn prepare_request(request: &Value, runs: &mut Runs) -> Result<(), crate::RpcError> {
    match request["method"].as_str() {
        Some("run.start" | "run.continue") => {
            runs.ensure_capacity().map_err(crate::RpcError::from_cause)
        }
        Some("run.recover") => {
            let params: RecoverRunParams =
                parameters(request.get("params").cloned().unwrap_or(json!({})))?;
            runs.ensure_recoverable(params.run_id)
                .map_err(crate::RpcError::from_cause)
        }
        _ => Ok(()),
    }
}

fn finish_request(
    request: &Value,
    result: Result<Value, crate::RpcError>,
    runs: &mut Runs,
) -> anyhow::Result<Option<Value>> {
    // Starting and recovering run ownership occurs only after the serialized
    // durable operation succeeded, including for JSON-RPC notifications.
    let result = match result {
        Ok(value) => Ok(match request["method"].as_str() {
            Some("run.start" | "run.continue") => json!(runs.start(serde_json::from_value(value)?)),
            Some("run.recover") => json!(runs.recovered(serde_json::from_value(value)?)),
            Some("run.status") => json!(runs.status(serde_json::from_value(value)?)),
            Some("run.list") => {
                let records: Vec<waypoints_core::run::Run> = serde_json::from_value(value)?;
                json!(
                    records
                        .into_iter()
                        .map(|run| runs.status(run))
                        .collect::<Vec<_>>()
                )
            }
            _ => value,
        }),
        Err(error) => Err(error),
    };
    Ok(reply(request, result))
}

async fn dispatch(request: &Value, storage: &mut Storage) -> Result<Value, crate::RpcError> {
    let method = request["method"]
        .as_str()
        .expect("validated request method");
    let params = request.get("params").cloned().unwrap_or(json!({}));
    match method {
        "run.start" => match parameters::<StartRunParams>(params) {
            Ok(params) => storage
                .create_run(params.workflow_id, params.node_id, &params.executable)
                .await
                .map(|run| json!(run))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "run.continue" => match parameters::<ContinueRunParams>(params) {
            Ok(params) => storage
                .continue_run(params.run_id, &params.prompt)
                .await
                .map(|run| json!(run))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "run.recovery" => match parameters::<RunParams>(params) {
            Ok(params) => storage
                .inspect_run_recovery(params.run_id)
                .await
                .map(|inspection| json!(inspection))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "run.recover" => match parameters::<RecoverRunParams>(params) {
            Ok(params) => storage
                .recover_run(params.run_id, params.confirmation)
                .await
                .map(|status| json!(status))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "run.inputs" => match parameters::<RunParams>(params) {
            Ok(params) => storage
                .run_upstream_status(params.run_id)
                .await
                .map(|status| json!(status))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "run.status" => match parameters::<RunParams>(params) {
            Ok(params) => storage
                .run(params.run_id)
                .await
                .map(|run| json!(run))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "run.list" => match parameters::<NodeParams>(params) {
            Ok(params) => storage
                .node_runs(params.workflow_id, params.node_id)
                .await
                .map(|records| json!(records))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "history.status" => match parameters::<ProjectParams>(params) {
            Ok(params) => storage
                .history_status(&params.project_path)
                .await
                .map(|status| json!(status))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "history.undo" | "history.redo" => match parameters::<HistoryParams>(params) {
            Ok(params) => {
                let result = if method == "history.undo" {
                    storage
                        .undo(&params.project_path, params.expected_edit_id)
                        .await
                } else {
                    storage
                        .redo(&params.project_path, params.expected_edit_id)
                        .await
                };
                result
                    .map(|status| json!(status))
                    .map_err(crate::RpcError::from_cause)
            }
            Err(error) => Err(error),
        },
        "repository.inspect" => match parameters::<ProjectParams>(params) {
            Ok(params) => waypoints_core::repository::inspect(&params.project_path)
                .await
                .map(|inspection| json!(inspection))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "workflow.create" => match parameters::<CreateParams>(params) {
            Ok(params) => {
                let result = match params.base {
                    Some(source) => {
                        storage
                            .create_workflow_from(&params.project_path, &params.name, &source)
                            .await
                    }
                    None => {
                        storage
                            .create_workflow(&params.project_path, &params.name)
                            .await
                    }
                };
                result
                    .map(|workflow| json!(workflow))
                    .map_err(crate::RpcError::from_cause)
            }
            Err(error) => Err(error),
        },
        "workflow.adopt_roots" => match parameters::<AdoptRootsParams>(params) {
            Ok(params) => storage
                .adopt_workflow_roots(params.workflow_id, &params.base_nodes)
                .await
                .map(|graph| json!(graph))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "workflow.rename" => match parameters::<RenameWorkflowParams>(params) {
            Ok(params) => storage
                .rename_workflow(params.workflow_id, &params.name)
                .await
                .map(|workflow| json!(workflow))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "workflow.archive" | "workflow.unarchive" => match parameters::<WorkflowParams>(params) {
            Ok(params) => {
                let result = if method == "workflow.archive" {
                    storage.archive_workflow(params.workflow_id).await
                } else {
                    storage.unarchive_workflow(params.workflow_id).await
                };
                result
                    .map(|workflow| json!(workflow))
                    .map_err(crate::RpcError::from_cause)
            }
            Err(error) => Err(error),
        },
        "workflow.get" => match parameters::<WorkflowParams>(params) {
            Ok(params) => storage
                .workflow(params.workflow_id)
                .await
                .map(|workflow| json!(workflow))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "workflow.set_base" => match parameters::<WorkflowBaseParams>(params) {
            Ok(params) => storage
                .set_workflow_base(params.workflow_id, &params.source)
                .await
                .map(|workflow| json!(workflow))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "workflow.list" => match parameters::<crate::protocol::WorkflowListParams>(params) {
            Ok(params) => storage
                .list_workflows_filtered(&params.project_path, params.filter)
                .await
                .map(|workflows| json!(workflows))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "workflow.graph" => match parameters::<WorkflowParams>(params) {
            Ok(params) => storage
                .workflow_graph(params.workflow_id)
                .await
                .map(|graph| json!(graph))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "node.resume_deletion" => match parameters::<RecoverDependencyParams>(params) {
            Ok(params) => storage
                .resume_node_deletion(params.operation_id, params.confirmation)
                .await
                .map(|operation| json!(operation))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "node.move_prompt" => match parameters::<crate::protocol::MovePromptParams>(params) {
            Ok(params) => storage
                .move_prompt_node(params.workflow_id, params.node_id, &params.parent)
                .await
                .map(|graph| json!(graph))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "node.delete_prompt" => match parameters::<NodeParams>(params) {
            Ok(params) => storage
                .delete_prompt_node(params.workflow_id, params.node_id)
                .await
                .map(|graph| json!(graph))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "node.create" => match parameters::<CreateNodeParams>(params) {
            Ok(params) if params.parent.is_some() && params.parents.is_some() => Err(
                crate::RpcError::new(-32602, "select parent or parents, not both".to_owned()),
            ),
            Ok(params) => {
                let result = match (params.parent, params.parents) {
                    (_, Some(parents)) => {
                        storage
                            .create_node_from_parents(params.workflow_id, &params.prompt, &parents)
                            .await
                    }
                    (Some(parent), None) => {
                        storage
                            .create_node_from(params.workflow_id, &params.prompt, &parent)
                            .await
                    }
                    (None, None) => {
                        storage
                            .create_node(params.workflow_id, &params.prompt)
                            .await
                    }
                };
                result
                    .map(|node| json!(node))
                    .map_err(crate::RpcError::from_cause)
            }
            Err(error) => Err(error),
        },
        "node.get" => match parameters::<NodeParams>(params) {
            Ok(params) => storage
                .node(params.workflow_id, params.node_id)
                .await
                .map(|node| json!(node))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "node.associate" => match parameters::<AssociateParams>(params) {
            Ok(params) => storage
                .associate_node(params.workflow_id, params.node_id, &params.change_id)
                .await
                .map(|state| json!(state))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "node.operation_recovery" | "node.reparent_recovery" => {
            match parameters::<DependencyOperationParams>(params) {
                Ok(params) => storage
                    .inspect_dependency_recovery(params.operation_id)
                    .await
                    .map(|value| json!(value))
                    .map_err(crate::RpcError::from_cause),
                Err(error) => Err(error),
            }
        }
        "node.recover_operation" | "node.recover_reparent" => {
            match parameters::<RecoverDependencyParams>(params) {
                Ok(params) => storage
                    .recover_dependency_operation(params.operation_id, params.confirmation)
                    .await
                    .map(|value| json!(value))
                    .map_err(crate::RpcError::from_cause),
                Err(error) => Err(error),
            }
        }
        "node.operations" => match parameters::<WorkflowParams>(params) {
            Ok(params) => storage
                .dependency_operations(params.workflow_id)
                .await
                .map(|value| json!(value))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "node.delete" => match parameters::<DeleteNodeParams>(params) {
            Ok(params) => storage
                .delete_node(params.workflow_id, params.node_id, params.acknowledgement)
                .await
                .map(|value| json!(value))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "node.reparent" => match parameters::<ReparentParams>(params) {
            Ok(params) => storage
                .reparent_node(
                    params.workflow_id,
                    params.node_id,
                    &params.parent,
                    params.acknowledgement,
                )
                .await
                .map(|value| json!(value))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "node.create_change" => match parameters::<CreateChangeParams>(params) {
            Ok(params) => {
                let result = match params.base {
                    Some(base) => {
                        storage
                            .create_node_change(params.workflow_id, params.node_id, &base)
                            .await
                    }
                    None => {
                        storage
                            .create_node_change_from_dependency(params.workflow_id, params.node_id)
                            .await
                    }
                };
                result
                    .map(|state| json!(state))
                    .map_err(crate::RpcError::from_cause)
            }
            Err(error) => Err(error),
        },
        "node.workspace.create" => match parameters::<CreateWorkspaceParams>(params) {
            Ok(params) => storage
                .create_node_workspace(params.workflow_id, params.node_id, &params.directory)
                .await
                .map(|state| json!(state))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "node.workspace.update_stale" => match parameters::<NodeParams>(params) {
            Ok(params) => storage
                .update_stale_node_workspace(params.workflow_id, params.node_id)
                .await
                .map(|state| json!(state))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "node.workspace.verify" => match parameters::<NodeParams>(params) {
            Ok(params) => storage
                .verify_node_workspace(params.workflow_id, params.node_id)
                .await
                .map(|state| json!(state))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "node.workspace.status" => match parameters::<NodeParams>(params) {
            Ok(params) => storage
                .node_workspace(params.workflow_id, params.node_id)
                .await
                .map(|state| json!(state))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "node.revisions" => match parameters::<NodeRevisionsParams>(params) {
            Ok(params) => storage
                .node_revisions(
                    params.workflow_id,
                    params.node_id,
                    params.page_size,
                    params.cursor.as_deref(),
                )
                .await
                .map(|page| json!(page))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "node.restore_with_descendants" => match parameters::<RestoreDescendantsParams>(params) {
            Ok(params) => storage
                .restore_node_with_descendants(&params.preview, params.confirmation)
                .await
                .map(|operation| json!(operation))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "node.restore_impact" => match parameters::<NodeRevisionParams>(params) {
            Ok(params) => storage
                .node_restore_impact(params.workflow_id, params.node_id, &params.commit_id)
                .await
                .map(|impact| json!(impact))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "node.restore_revision" => match parameters::<NodeRevisionParams>(params) {
            Ok(params) => storage
                .restore_node_revision(params.workflow_id, params.node_id, &params.commit_id)
                .await
                .map(|operation| json!(operation))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "node.restores" => match parameters::<NodeParams>(params) {
            Ok(params) => storage
                .node_restores(params.workflow_id, params.node_id)
                .await
                .map(|operations| json!(operations))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "node.revision_review" => match parameters::<NodeRevisionParams>(params) {
            Ok(params) => storage
                .review_node_revision(params.workflow_id, params.node_id, &params.commit_id)
                .await
                .map(|review| json!(review))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "node.review" => match parameters::<NodeParams>(params) {
            Ok(params) => storage
                .review_node(params.workflow_id, params.node_id)
                .await
                .map(|review| json!(review))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "run.restore" => match parameters::<RunParams>(params) {
            Ok(params) => storage
                .restore_run(params.run_id)
                .await
                .map(|operation| json!(operation))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "run.restore_status" => match parameters::<RunParams>(params) {
            Ok(params) => storage
                .run_restores(params.run_id)
                .await
                .map(|operations| json!(operations))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "run.recover_restore" => match parameters::<RecoverDependencyParams>(params) {
            Ok(params) => storage
                .recover_restore(params.operation_id, params.confirmation)
                .await
                .map(|operation| json!(operation))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "run.review" => match parameters::<RunParams>(params) {
            Ok(params) => storage
                .review_run(params.run_id)
                .await
                .map(|review| json!(review))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "node.refresh" => match parameters::<NodeParams>(params) {
            Ok(params) => storage
                .refresh_node(params.workflow_id, params.node_id)
                .await
                .map(|state| json!(state))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "node.update" => match parameters::<UpdateNodeParams>(params) {
            Ok(params) => storage
                .update_node(params.workflow_id, params.node_id, &params.prompt)
                .await
                .map(|node| json!(node))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        "connection.create" => match parameters::<ConnectParams>(params) {
            Ok(params) => storage
                .connect_nodes(params.workflow_id, params.source_id, params.target_id)
                .await
                .map(|connection| json!(connection))
                .map_err(crate::RpcError::from_cause),
            Err(error) => Err(error),
        },
        _ => Err(crate::RpcError::new(-32601, "method not found".to_owned())),
    }
}

fn parameters<T: DeserializeOwned>(params: Value) -> Result<T, crate::RpcError> {
    serde_json::from_value(params)
        .map_err(|error| crate::RpcError::new(-32602, format!("invalid parameters: {error}")))
}
