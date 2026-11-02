use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader};
use waypoints_core::run::RecoveryConfirmation;
use waypoints_core::storage::{UndoBoundaryAcknowledgement, WorkflowFilter};
use waypoints_core::workflow::{BaseSource, NodeParent};

pub(crate) const TIMEOUT: Duration = Duration::from_secs(5);
// Bound memory usage even when a client sends a frame without a newline.
pub(crate) const MAX_FRAME: usize = 1024 * 1024;
pub(crate) const PROTOCOL_VERSION: u32 = 1;

/// A remote JSON-RPC failure, preserving unknown and legacy server details.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    code: i32,
    message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    data: Option<serde_json::Value>,
}

impl RpcError {
    pub(crate) fn new(code: i32, message: String) -> Self {
        Self {
            code,
            message,
            data: None,
        }
    }

    pub(crate) fn busy() -> Self {
        Self {
            code: -32001,
            message: "backend storage operation in progress; request was not started".to_owned(),
            data: Some(serde_json::json!({"kind":"backend_busy"})),
        }
    }

    /// Whether the backend explicitly rejected this request before starting it.
    ///
    /// This is not a transport timeout and does not describe requests whose
    /// response was lost after execution started.
    pub fn is_busy(&self) -> bool {
        self.code == -32001
            && self.data.as_ref().and_then(|data| data["kind"].as_str()) == Some("backend_busy")
    }

    /// The original JSON-RPC error code, including standard validation codes.
    pub fn code(&self) -> i32 {
        self.code
    }

    /// The server's human-readable diagnostic; not a classification API.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The original optional data, including unknown future server schemas.
    pub fn data(&self) -> Option<&serde_json::Value> {
        self.data.as_ref()
    }

    /// Decodes only recognized explicit storage refusals, never English messages.
    pub fn failure(&self) -> Option<waypoints_core::storage::StorageFailure> {
        if self.code != -32000 {
            return None;
        }
        serde_json::from_value(self.data.as_ref()?.clone()).ok()
    }

    pub(crate) fn from_cause(cause: impl Into<anyhow::Error>) -> Self {
        let cause = cause.into();
        let failure = cause
            .chain()
            .find_map(|cause| cause.downcast_ref::<waypoints_core::storage::StorageFailure>());
        Self {
            code: -32000,
            message: format!("{cause:#}"),
            data: failure.map(|failure| serde_json::json!(failure)),
        }
    }
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Retain the CLI's existing JSON-quoted remote message presentation.
        write!(
            formatter,
            "backend error {}: {}",
            self.code,
            serde_json::json!(self.message)
        )
    }
}

impl std::error::Error for RpcError {}

#[cfg(test)]
mod error_tests {
    use super::*;
    use waypoints_core::storage::StorageFailure;

    #[test]
    fn remote_errors_preserve_legacy_and_unknown_data_without_guessing_categories()
    -> anyhow::Result<()> {
        for data in [
            None,
            Some(serde_json::json!({"kind":"future_failure","detail":[1,2]})),
        ] {
            let mut value =
                serde_json::json!({"code":-32000,"message":"node is reserved by run 7"});
            if let Some(data) = &data {
                value["data"] = data.clone();
            }
            let error: RpcError = serde_json::from_value(value.clone())?;
            assert_eq!(error.data(), data.as_ref());
            assert!(error.failure().is_none(), "diagnostics must not be parsed");
            assert!(!error.is_busy());
            assert_eq!(serde_json::to_value(error)?, value);
        }
        let cause = anyhow::Error::new(StorageFailure::MissingRun { run_id: 7 })
            .context("while inspecting");
        let error = RpcError::from_cause(cause);
        assert_eq!(
            error.failure(),
            Some(StorageFailure::MissingRun { run_id: 7 })
        );
        let error = RpcError::busy();
        assert!(error.is_busy());
        assert!(error.failure().is_none());
        Ok(())
    }
}

// Native work and full-graph verification have bounded longer deadlines.
// Ordinary requests keep their short deadline.
pub(crate) fn request_timeout(method: &str) -> Duration {
    if matches!(
        method,
        "node.workspace.create"
            | "node.workspace.update_stale"
            | "node.reparent"
            | "node.delete"
            | "node.operation_recovery"
            | "node.recover_operation"
            | "node.reparent_recovery"
            | "node.recover_reparent"
            | "node.resume_deletion"
            | "run.restore"
            | "node.restore_revision"
            | "node.restore_impact"
            | "node.restore_with_descendants"
            | "run.recover_restore"
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
pub(crate) struct WorkflowListParams {
    pub project_path: PathBuf,
    #[serde(default)]
    pub filter: WorkflowFilter,
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
pub(crate) struct NodeRevisionsParams {
    pub workflow_id: i64,
    pub node_id: i64,
    pub page_size: usize,
    pub cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NodeRevisionParams {
    pub workflow_id: i64,
    pub node_id: i64,
    pub commit_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RestoreDescendantsParams {
    pub preview: waypoints_core::storage::RestoreImpact,
    pub confirmation: Option<waypoints_core::storage::RestoreRebaseConfirmation>,
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
