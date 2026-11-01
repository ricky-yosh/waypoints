use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader};
use waypoints_core::run::RecoveryConfirmation;
use waypoints_core::storage::UndoBoundaryAcknowledgement;
use waypoints_core::workflow::{BaseSource, NodeParent};

pub(crate) const TIMEOUT: Duration = Duration::from_secs(5);
// Bound memory usage even when a client sends a frame without a newline.
pub(crate) const MAX_FRAME: usize = 1024 * 1024;
pub(crate) const PROTOCOL_VERSION: u32 = 1;

// Native work and full-graph verification have bounded longer deadlines.
// Ordinary requests keep their short deadline.
pub(crate) fn request_timeout(method: &str) -> Duration {
    if matches!(
        method,
        "node.workspace.create"
            | "node.reparent"
            | "node.delete"
            | "node.operation_recovery"
            | "node.recover_operation"
            | "node.reparent_recovery"
            | "node.recover_reparent"
            | "node.resume_deletion"
    ) {
        Duration::from_secs(120)
    } else {
        TIMEOUT
    }
}

/// The identity returned by a running backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BackendInfo {
    /// The server process identifier.
    pub pid: u32,
    /// The canonical database path owned by the server.
    pub database: PathBuf,
    /// The application protocol version, separate from JSON-RPC 2.0.
    pub protocol_version: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProjectParams {
    pub project_path: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HistoryParams {
    pub project_path: PathBuf,
    pub expected_edit_id: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateParams {
    pub project_path: PathBuf,
    pub name: String,
    #[serde(default)]
    pub base: Option<BaseSource>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkflowBaseParams {
    pub workflow_id: i64,
    pub source: BaseSource,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AdoptRootsParams {
    pub workflow_id: i64,
    pub base_nodes: Vec<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkflowParams {
    pub workflow_id: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RenameWorkflowParams {
    pub workflow_id: i64,
    pub name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NodeParams {
    pub workflow_id: i64,
    pub node_id: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StartRunParams {
    pub workflow_id: i64,
    pub node_id: i64,
    pub executable: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContinueRunParams {
    pub run_id: i64,
    pub prompt: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunParams {
    pub run_id: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecoverRunParams {
    pub run_id: i64,
    pub confirmation: RecoveryConfirmation,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunEventsParams {
    pub run_id: i64,
    pub after: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PermissionParams {
    pub run_id: i64,
    pub permission_id: u64,
    pub option_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AssociateParams {
    pub workflow_id: i64,
    pub node_id: i64,
    pub change_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateChangeParams {
    pub workflow_id: i64,
    pub node_id: i64,
    #[serde(default)]
    pub base: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReparentParams {
    pub workflow_id: i64,
    pub node_id: i64,
    pub parent: NodeParent,
    #[serde(default)]
    pub acknowledgement: Option<UndoBoundaryAcknowledgement>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MovePromptParams {
    pub workflow_id: i64,
    pub node_id: i64,
    pub parent: NodeParent,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeleteNodeParams {
    pub workflow_id: i64,
    pub node_id: i64,
    #[serde(default)]
    pub acknowledgement: Option<UndoBoundaryAcknowledgement>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DependencyOperationParams {
    pub operation_id: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecoverDependencyParams {
    pub operation_id: i64,
    #[serde(default)]
    pub confirmation: Option<RecoveryConfirmation>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateWorkspaceParams {
    pub workflow_id: i64,
    pub node_id: i64,
    pub directory: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateNodeParams {
    pub workflow_id: i64,
    pub prompt: String,
    #[serde(default)]
    pub parent: Option<NodeParent>,
    #[serde(default, deserialize_with = "node_parents")]
    pub parents: Option<Vec<NodeParent>>,
}

fn node_parents<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<NodeParent>>, D::Error> {
    Vec::<NodeParent>::deserialize(deserializer).map(Some)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateNodeParams {
    pub workflow_id: i64,
    pub node_id: i64,
    pub prompt: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConnectParams {
    pub workflow_id: i64,
    pub source_id: i64,
    pub target_id: i64,
}

pub(crate) async fn read_frame<R: AsyncRead + Unpin>(
    stream: &mut BufReader<R>,
) -> anyhow::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    (&mut *stream)
        .take(MAX_FRAME as u64 + 1)
        .read_until(b'\n', &mut bytes)
        .await
        .context("reading JSON-RPC frame")?;
    anyhow::ensure!(bytes.len() <= MAX_FRAME, "JSON-RPC frame exceeds 1 MiB");
    anyhow::ensure!(bytes.last() == Some(&b'\n'), "incomplete JSON-RPC frame");
    Ok(bytes)
}
