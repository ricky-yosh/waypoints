use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::time::timeout;
use waypoints_core::storage::Storage;

use crate::config::sidecar;
use crate::protocol::{
    AssociateParams, ConnectParams, ContinueRunParams, CreateChangeParams, CreateNodeParams,
    CreateParams, CreateWorkspaceParams, HistoryParams, MAX_FRAME, NodeParams, PROTOCOL_VERSION,
    PermissionParams, ProjectParams, RecoverRunParams, RunEventsParams, RunParams, StartRunParams,
    TIMEOUT, UpdateNodeParams, WorkflowBaseParams, WorkflowParams, read_frame,
};
use crate::runs::Runs;
use crate::{BackendError, BackendInfo, Config};

/// Runs the local server until a shutdown request or Ctrl+C.
///
/// Requests are serialized. Each connection carries one newline-delimited JSON-RPC
/// request, bounded to 1 MiB. Ownership locks are acquired before opening storage;
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
    let serving = async {
        loop {
            let stream = tokio::select! {
                Some((id, update)) = runs.rx.recv() => {
                    runs.apply(id, update, &mut storage).await?;
                    continue;
                }
                incoming = listener.accept() => incoming.context("accepting backend connection")?.0,
                signal = tokio::signal::ctrl_c() => {
                    signal.context("waiting for shutdown signal")?;
                    break;
                }
            };
            // Bad, disconnected, oversized, and slow clients cannot terminate the server.
            if matches!(
                handle(stream, &mut storage, &info, &mut runs).await,
                Ok(true)
            ) {
                break;
            }
        }
        Ok::<_, anyhow::Error>(())
    }
    .await;
    // Keep ownership locks until all owned tasks/processes are stopped.
    let cleanup = runs.shutdown(&mut storage).await;
    serving?;
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

async fn handle(
    stream: UnixStream,
    storage: &mut Storage,
    info: &BackendInfo,
    runs: &mut Runs,
) -> anyhow::Result<bool> {
    let mut stream = BufReader::new(stream);
    let bytes = timeout(TIMEOUT, read_frame(&mut stream))
        .await
        .context("client request timed out")??;
    let (response, stop) = match serde_json::from_slice::<Value>(&bytes) {
        Ok(request) => dispatch(request, storage, info, runs).await,
        Err(_) => (Some(error(Value::Null, -32700, "parse error")), false),
    };
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
        timeout(TIMEOUT, stream.get_mut().write_all(&bytes))
            .await
            .context("client response timed out")??;
    }
    Ok(stop)
}

fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

async fn dispatch(
    request: Value,
    storage: &mut Storage,
    info: &BackendInfo,
    runs: &mut Runs,
) -> (Option<Value>, bool) {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let valid_id = id.is_null() || id.is_string() || id.is_number();
    let Some(method) = request
        .get("method")
        .and_then(Value::as_str)
        .filter(|_| request["jsonrpc"] == "2.0" && valid_id)
    else {
        return (Some(error(Value::Null, -32600, "invalid request")), false);
    };
    let notification = request.get("id").is_none();
    let params = request.get("params").cloned().unwrap_or(json!({}));
    let mut stop = false;
    let result: Result<Value, (i32, String)> = match method {
        "backend.info" | "backend.shutdown" => {
            if params != json!({}) {
                Err((-32602, "expected empty parameters".to_owned()))
            } else if method == "backend.shutdown" {
                stop = true;
                Ok(Value::Null)
            } else {
                Ok(json!(info))
            }
        }
        "run.start" => match parameters::<StartRunParams>(params) {
            Ok(params) => async {
                runs.ensure_capacity()?;
                let run = storage
                    .create_run(params.workflow_id, params.node_id, &params.executable)
                    .await?;
                Ok::<_, anyhow::Error>(json!(runs.start(run)))
            }
            .await
            .map_err(|error| (-32000, format!("{error:#}"))),
            Err(error) => Err(error),
        },
        "run.continue" => match parameters::<ContinueRunParams>(params) {
            Ok(params) => async {
                runs.ensure_capacity()?;
                let run = storage.continue_run(params.run_id, &params.prompt).await?;
                Ok::<_, anyhow::Error>(json!(runs.start(run)))
            }
            .await
            .map_err(|error| (-32000, format!("{error:#}"))),
            Err(error) => Err(error),
        },
        "run.recovery" => match parameters::<RunParams>(params) {
            Ok(params) => storage
                .inspect_run_recovery(params.run_id)
                .await
                .map(|inspection| json!(inspection))
                .map_err(|error| (-32000, format!("{error:#}"))),
            Err(error) => Err(error),
        },
        "run.recover" => match parameters::<RecoverRunParams>(params) {
            Ok(params) => runs
                .recover(params.run_id, params.confirmation, storage)
                .await
                .map(|status| json!(status))
                .map_err(|error| (-32000, format!("{error:#}"))),
            Err(error) => Err(error),
        },
        "run.status" => match parameters::<RunParams>(params) {
            Ok(params) => storage
                .run(params.run_id)
                .await
                .map(|run| json!(runs.status(run)))
                .map_err(|error| (-32000, format!("{error:#}"))),
            Err(error) => Err(error),
        },
        "run.list" => match parameters::<NodeParams>(params) {
            Ok(params) => storage
                .node_runs(params.workflow_id, params.node_id)
                .await
                .map(|records| {
                    json!(
                        records
                            .into_iter()
                            .map(|run| runs.status(run))
                            .collect::<Vec<_>>()
                    )
                })
                .map_err(|error| (-32000, format!("{error:#}"))),
            Err(error) => Err(error),
        },
        "run.events" => match parameters::<RunEventsParams>(params) {
            Ok(params) => runs
                .events(params.run_id, params.after)
                .map(|events| json!(events))
                .map_err(|error| (-32000, format!("{error:#}"))),
            Err(error) => Err(error),
        },
        "run.cancel" => match parameters::<RunParams>(params) {
            Ok(params) => runs
                .cancel(params.run_id, storage)
                .await
                .map(|status| json!(status))
                .map_err(|error| (-32000, format!("{error:#}"))),
            Err(error) => Err(error),
        },
        "run.permission" => match parameters::<PermissionParams>(params) {
            Ok(params) => runs
                .respond(
                    params.run_id,
                    params.permission_id,
                    params.option_id,
                    storage,
                )
                .await
                .map(|status| json!(status))
                .map_err(|error| (-32000, format!("{error:#}"))),
            Err(error) => Err(error),
        },
        "history.status" => match parameters::<ProjectParams>(params) {
            Ok(params) => storage
                .history_status(&params.project_path)
                .await
                .map(|status| json!(status))
                .map_err(|error| (-32000, format!("{error:#}"))),
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
                    .map_err(|error| (-32000, format!("{error:#}")))
            }
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
                    .map_err(|error| (-32000, format!("{error:#}")))
            }
            Err(error) => Err(error),
        },
        "workflow.get" => match parameters::<WorkflowParams>(params) {
            Ok(params) => storage
                .workflow(params.workflow_id)
                .await
                .map(|workflow| json!(workflow))
                .map_err(|error| (-32000, format!("{error:#}"))),
            Err(error) => Err(error),
        },
        "workflow.set_base" => match parameters::<WorkflowBaseParams>(params) {
            Ok(params) => storage
                .set_workflow_base(params.workflow_id, &params.source)
                .await
                .map(|workflow| json!(workflow))
                .map_err(|error| (-32000, format!("{error:#}"))),
            Err(error) => Err(error),
        },
        "workflow.list" => match parameters::<ProjectParams>(params) {
            Ok(params) => storage
                .list_workflows(&params.project_path)
                .await
                .map(|workflows| json!(workflows))
                .map_err(|error| (-32000, format!("{error:#}"))),
            Err(error) => Err(error),
        },
        "workflow.graph" => match parameters::<WorkflowParams>(params) {
            Ok(params) => storage
                .workflow_graph(params.workflow_id)
                .await
                .map(|graph| json!(graph))
                .map_err(|error| (-32000, format!("{error:#}"))),
            Err(error) => Err(error),
        },
        "node.create" => match parameters::<CreateNodeParams>(params) {
            Ok(params) => storage
                .create_node(params.workflow_id, &params.prompt)
                .await
                .map(|node| json!(node))
                .map_err(|error| (-32000, format!("{error:#}"))),
            Err(error) => Err(error),
        },
        "node.get" => match parameters::<NodeParams>(params) {
            Ok(params) => storage
                .node(params.workflow_id, params.node_id)
                .await
                .map(|node| json!(node))
                .map_err(|error| (-32000, format!("{error:#}"))),
            Err(error) => Err(error),
        },
        "node.associate" => match parameters::<AssociateParams>(params) {
            Ok(params) => storage
                .associate_node(params.workflow_id, params.node_id, &params.change_id)
                .await
                .map(|state| json!(state))
                .map_err(|error| (-32000, format!("{error:#}"))),
            Err(error) => Err(error),
        },
        "node.create_change" => match parameters::<CreateChangeParams>(params) {
            Ok(params) => storage
                .create_node_change(params.workflow_id, params.node_id, &params.base)
                .await
                .map(|state| json!(state))
                .map_err(|error| (-32000, format!("{error:#}"))),
            Err(error) => Err(error),
        },
        "node.workspace.create" => match parameters::<CreateWorkspaceParams>(params) {
            Ok(params) => storage
                .create_node_workspace(params.workflow_id, params.node_id, &params.directory)
                .await
                .map(|state| json!(state))
                .map_err(|error| (-32000, format!("{error:#}"))),
            Err(error) => Err(error),
        },
        "node.workspace.verify" => match parameters::<NodeParams>(params) {
            Ok(params) => storage
                .verify_node_workspace(params.workflow_id, params.node_id)
                .await
                .map(|state| json!(state))
                .map_err(|error| (-32000, format!("{error:#}"))),
            Err(error) => Err(error),
        },
        "node.workspace.status" => match parameters::<NodeParams>(params) {
            Ok(params) => storage
                .node_workspace(params.workflow_id, params.node_id)
                .await
                .map(|state| json!(state))
                .map_err(|error| (-32000, format!("{error:#}"))),
            Err(error) => Err(error),
        },
        "node.refresh" => match parameters::<NodeParams>(params) {
            Ok(params) => storage
                .refresh_node(params.workflow_id, params.node_id)
                .await
                .map(|state| json!(state))
                .map_err(|error| (-32000, format!("{error:#}"))),
            Err(error) => Err(error),
        },
        "node.update" => match parameters::<UpdateNodeParams>(params) {
            Ok(params) => storage
                .update_node(params.workflow_id, params.node_id, &params.prompt)
                .await
                .map(|node| json!(node))
                .map_err(|error| (-32000, format!("{error:#}"))),
            Err(error) => Err(error),
        },
        "connection.create" => match parameters::<ConnectParams>(params) {
            Ok(params) => storage
                .connect_nodes(params.workflow_id, params.source_id, params.target_id)
                .await
                .map(|connection| json!(connection))
                .map_err(|error| (-32000, format!("{error:#}"))),
            Err(error) => Err(error),
        },
        _ => Err((-32601, "method not found".to_owned())),
    };
    let response = match result {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err((code, message)) => error(id, code, &message),
    };
    ((!notification).then_some(response), stop)
}

fn parameters<T: DeserializeOwned>(params: Value) -> Result<T, (i32, String)> {
    serde_json::from_value(params).map_err(|error| (-32602, format!("invalid parameters: {error}")))
}
