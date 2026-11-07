//! Durable references and outcomes for backend-owned agent runs, without transcripts.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::jj::{self, Revision};
use crate::storage::StorageError;
use crate::workspace::WorkspaceAssociation;

// Includes native revalidation and the snapshot command, not database writes.
const SNAPSHOT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// A captured agent turn and its outcome, linked to a node and harness session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Run {
    /// Database-local identifier, never reused after a backend restart.
    pub id: i64,
    /// Workflow containing the node.
    pub workflow_id: i64,
    /// Node owning the run.
    pub node_id: i64,
    /// Captured node assignment or explicit follow-up; later edits cannot change it.
    pub prompt: String,
    /// Dedicated workspace selected at start time.
    pub directory: PathBuf,
    /// Native change selected at start time.
    pub change_id: String,
    /// Explicitly selected, absolute ACP executable; never interpreted by a shell.
    pub executable: PathBuf,
    /// Harness-owned session reference, not a copy of its conversation.
    pub session_id: Option<String>,
    /// Selected previous run for explicit continuation, not a conversation fork.
    #[serde(default)]
    pub continued_from: Option<i64>,
    /// Adapter process/group ID for diagnosis; never signal a saved PID after restart.
    pub process_id: Option<u32>,
    /// Last persisted lifecycle state.
    pub state: RunState,
    /// Native revision recording, independent of the agent execution outcome.
    #[serde(default)]
    pub snapshot: SnapshotState,
    /// Direct saved graph dependencies observed before this turn; absent for legacy/unavailable inputs.
    #[serde(default)]
    pub upstream_inputs: Option<Vec<RunInput>>,
}

/// A bounded, newest-first page of saved runs, pinned against later insertions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RunPage {
    /// Saved runs, including their captured instructions.
    pub runs: Vec<Run>,
    /// Opaque, node-scoped cursor for older records.
    pub next_cursor: Option<String>,
}

/// The newest saved run for one visible workflow node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct NodeRun {
    /// Node identity, including nodes which have never run.
    pub node_id: i64,
    /// Latest run, or none when the node has no history.
    pub latest: Option<Run>,
}

/// A bounded workflow-wide run overview, ordered by node ID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct WorkflowRunPage {
    /// Latest saved state for each node on this page.
    pub nodes: Vec<NodeRun>,
    /// Last included node; use as `after_node` for the next page.
    pub next_node: Option<i64>,
}

/// Readonly assessment of a saved session; execution always revalidates it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ContinuationAssessment {
    /// Local checks passed, but the adapter must still support loading the session.
    Eligible { run_id: i64 },
    /// A known local prerequisite prevents continuation.
    Blocked {
        /// Machine-readable cause, independent of diagnostic wording.
        reason: ContinuationBlocker,
        /// Diagnostic with the relevant recovery instruction.
        detail: String,
    },
}

/// Known reasons a saved session cannot be continued.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ContinuationBlocker {
    RunNotStopped,
    MissingSession,
    RestoredSession,
    Superseded,
    NodeBusy,
    WorkspaceNotReady,
    WorkspaceChanged,
    ExecutableChanged,
}

/// One direct workflow dependency observed before an agent turn, not agent context provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RunInput {
    /// The saved base or upstream node selected by the graph.
    pub parent: crate::workflow::NodeParent,
    /// Exact recorded native revision observed for that dependency.
    pub commit_id: String,
}

/// Informational comparison of a run's saved upstream inputs with current graph dependencies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
#[non_exhaustive]
pub enum UpstreamStatus {
    /// Direct dependencies still have the same recorded commit IDs.
    Unchanged,
    /// At least one direct dependency or its recorded revision differs.
    Changed,
    /// Inputs were not recorded or cannot currently be compared.
    Unknown {
        /// Why no comparison can be made; never evidence that inputs are unchanged.
        detail: String,
    },
}

/// Whether a run's successful turn has a durably recorded native revision.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
#[non_exhaustive]
pub enum SnapshotState {
    /// Not recorded: active, cancelled, failed, or created before automatic snapshots.
    #[default]
    NotRecorded,
    /// Exact revision produced by native snapshotting; not approval of the work.
    Saved {
        /// Native change and commit identifiers, description, and conflict status.
        revision: Revision,
    },
    /// Agent completed, but no exact revision could be confirmed and saved.
    Failed {
        /// Failure may follow a native mutation; inspect jj rather than replaying work.
        detail: String,
    },
}

/// Last known lifecycle phase when run ownership was lost.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum InterruptionPhase {
    /// Legacy records did not persist the interrupted phase.
    #[default]
    Unknown,
    /// Agent execution or process cleanup had not been confirmed finished.
    Execution,
    /// Agent cleanup finished, but native snapshot recording was unfinished.
    Finalization,
}

/// Explicit caller attestation, not a conclusion drawn from a saved process ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RecoveryConfirmation {
    /// Caller checked that the old adapter, tools, and native jj writers stopped.
    ProcessesStopped,
}

/// Read-only recovery inspection; workspace verification cannot prove process exit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RunRecovery {
    /// Saved interruption/session/workspace metadata, not a reconstructed outcome.
    pub run: Run,
    /// Current workspace checks, separate from the interrupted run's output.
    pub workspace: RecoveryWorkspace,
}

/// Whether the current allocation passes read-only native checkout verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
#[non_exhaustive]
pub enum RecoveryWorkspace {
    /// Current workspace matches the saved run; process confirmation is still required.
    Verified {
        /// Observation only; never attributed to the interrupted run as its snapshot.
        current_revision: Revision,
    },
    /// Release is refused until native state can be safely verified.
    Blocked {
        /// Actionable native, allocation, or storage diagnostic.
        detail: String,
    },
}

/// Verified snapshot inputs, prepared by storage while the run retains its reservation.
#[derive(Debug)]
pub struct RunSnapshot {
    pub(crate) workspace: WorkspaceAssociation,
    pub(crate) revision: Revision,
}

impl RunSnapshot {
    /// Snapshots the verified workspace through jj without accessing the database.
    ///
    /// Native ignore/tracking rules apply. Local edits are neither cleaned nor
    /// reset; no child change, description, or review approval is created.
    ///
    /// # Errors
    ///
    /// Returns an error for a changed/stale checkout, native failure, unsupported
    /// output, or a 60-second deadline. Failure may follow a successful native
    /// snapshot; never automatically repeat it or claim a recorded revision.
    pub async fn record(self) -> Result<Revision, StorageError> {
        use anyhow::Context;

        Ok(tokio::time::timeout(
            SNAPSHOT_TIMEOUT,
            jj::snapshot_workspace(&self.workspace, &self.revision),
        )
        .await
        .context("run snapshot timed out; inspect native jj state")??)
    }
}

/// Lifecycle states; terminal does not imply that the work was snapshotted by jj.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
#[non_exhaustive]
pub enum RunState {
    /// Allocation saved, adapter initialization in progress.
    Starting,
    /// Agent is processing the captured prompt.
    Running,
    /// A client decision is required; no permission is automatically granted.
    AwaitingPermission,
    /// Cancellation requested; workspace remains reserved until cleanup finishes.
    Cancelling,
    /// Agent completed and cleanup was confirmed; snapshotting retains the reservation.
    Finalizing,
    /// Agent ended normally; inspect snapshot separately for revision recording failures.
    Completed,
    /// Run stopped on request or after permission expiry.
    Cancelled,
    /// Adapter failed; owned process cleanup completed.
    Failed {
        /// Actionable failure description.
        detail: String,
    },
    /// Backend lost ownership or could not prove cleanup; reservation is retained.
    Interrupted {
        /// Manual inspection required before any further editing.
        detail: String,
        /// Persisted across restarts; unknown on legacy interrupted records.
        #[serde(default)]
        phase: InterruptionPhase,
    },
    /// Caller confirmed old writers stopped and read-only workspace checks passed.
    Recovered {
        /// Original interruption diagnostic, retained rather than claiming success.
        detail: String,
        /// Original interrupted phase; no lost execution/snapshot outcome is invented.
        #[serde(default)]
        phase: InterruptionPhase,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn legacy_interrupted_phase_is_unknown() -> serde_json::Result<()> {
        let state: RunState = serde_json::from_value(json!({
            "status": "interrupted", "detail": "original legacy cause"
        }))?;
        assert_eq!(
            state,
            RunState::Interrupted {
                detail: "original legacy cause".into(),
                phase: InterruptionPhase::Unknown,
            }
        );
        Ok(())
    }

    #[test]
    fn reads_legacy_records_without_continuation_metadata() -> serde_json::Result<()> {
        let run: Run = serde_json::from_value(json!({
            "id": 1, "workflow_id": 1, "node_id": 1, "prompt": "saved assignment",
            "directory": "/workspace", "change_id": "saved-change",
            "executable": "/bin/agent", "session_id": "saved-session",
            "process_id": null, "state": {"status": "completed"}
        }))?;
        assert_eq!(run.continued_from, None);
        assert_eq!(run.session_id.as_deref(), Some("saved-session"));
        assert_eq!(run.state, RunState::Completed);
        assert_eq!(run.snapshot, SnapshotState::NotRecorded);
        assert!(run.upstream_inputs.is_none());
        Ok(())
    }
}
