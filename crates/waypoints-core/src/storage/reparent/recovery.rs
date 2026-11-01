//! Explicit, metadata-only recovery for two provable native dependency-edit outcomes.

use std::time::Duration;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use super::{
    DependencyOperation, DependencyOperationState, verify_graph_ancestry,
    verify_reserved_dependencies,
};
use crate::jj;
use crate::run::RecoveryConfirmation;
use crate::storage::{Storage, StorageError, WorkflowGraph};

// Recovery only queries jj; allow a bounded full-graph verification.
const RECOVERY_TIMEOUT: Duration = Duration::from_secs(40);

/// A read-only assessment, not proof that an old native process has stopped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
#[non_exhaustive]
pub enum DependencyRecoveryAssessment {
    /// Source was abandoned, but its saved workspace is still registered; explicit resume may forget it.
    DeletionAbandoned,
    /// The repository is still at the exact preflight operation and revisions.
    NotApplied,
    /// A single native operation produced the exact intended dependency graph.
    Applied,
    /// The saved operation already reached a terminal state.
    Resolved,
    /// Native or saved state is ambiguous; the reservation must remain.
    Blocked {
        /// Why automatic classification is unsafe.
        detail: String,
    },
}

/// Saved intent and freshly checked recovery eligibility without mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DependencyRecovery {
    /// The original durable operation, not a reconstructed native action.
    pub operation: DependencyOperation,
    /// Current eligibility; rechecked when explicit recovery is requested.
    pub assessment: DependencyRecoveryAssessment,
}

impl Storage {
    pub(super) async fn saved_dependency_operation(
        &self,
        id: i64,
    ) -> anyhow::Result<(String, DependencyOperation)> {
        let connection = self.database.connect()?;
        let mut rows = connection
            .query(
                "SELECT workflow_id, active, record FROM dependency_operations WHERE id = ?1",
                [id],
            )
            .await?;
        let row = rows
            .next()
            .await?
            .context("dependency operation not found")?;
        let record: String = row.get(2)?;
        let operation: DependencyOperation = serde_json::from_str(&record)?;
        anyhow::ensure!(
            operation.id == id && operation.workflow_id == row.get::<i64>(0)?,
            "dependency operation identity does not match its record"
        );
        anyhow::ensure!(
            (row.get::<i64>(1)? == 1) == (operation.state == DependencyOperationState::Unresolved),
            "dependency operation reservation does not match its state"
        );
        Ok((record, operation))
    }

    /// Inspects a saved dependency-edit outcome without changing jj, files, or metadata.
    ///
    /// # Errors
    ///
    /// Returns an error for missing/invalid storage records. Native uncertainty is
    /// reported as a blocked assessment and never releases the reservation.
    pub async fn inspect_dependency_recovery(
        &self,
        id: i64,
    ) -> Result<DependencyRecovery, StorageError> {
        let (_, operation) = self.saved_dependency_operation(id).await?;
        let assessment = if operation.state != DependencyOperationState::Unresolved {
            DependencyRecoveryAssessment::Resolved
        } else if operation.deletion_workspace.is_some()
            && self.verify_abandoned_deletion(&operation).await.is_ok()
        {
            DependencyRecoveryAssessment::DeletionAbandoned
        } else {
            match self.checked_dependency_resolution(&operation).await {
                Ok((_, resolved)) => match resolved.state {
                    DependencyOperationState::NotApplied => {
                        DependencyRecoveryAssessment::NotApplied
                    }
                    _ => DependencyRecoveryAssessment::Applied,
                },
                Err(error) => DependencyRecoveryAssessment::Blocked {
                    detail: format!("{error:#}"),
                },
            }
        };
        Ok(DependencyRecovery {
            operation,
            assessment,
        })
    }

    /// Resolves only a clearly applied or unapplied dependency edit, without rerunning jj.
    ///
    /// The caller must attest that old native processes and other writers stopped.
    /// This is not a force flag: fresh checks still refuse ambiguous state. A resolved
    /// operation is an idempotent no-op, even if newer work now holds a reservation.
    /// The original undo boundary and all files/run records remain unchanged.
    ///
    /// # Errors
    ///
    /// Returns an error for missing confirmation, uncertainty, or persistence failure.
    /// Failed writes retain the reservation and roll back any graph reconciliation.
    pub async fn recover_dependency_operation(
        &mut self,
        id: i64,
        confirmation: Option<RecoveryConfirmation>,
    ) -> Result<DependencyOperation, StorageError> {
        if confirmation != Some(RecoveryConfirmation::ProcessesStopped) {
            return Err(anyhow::anyhow!(
                "confirm all old native processes and other writers have stopped with --confirm-processes-stopped; recovery never reruns jj"
            ).into());
        }
        let (record, operation) = self.saved_dependency_operation(id).await?;
        if operation.state != DependencyOperationState::Unresolved {
            return Ok(operation);
        }
        let (graph, resolved) = self
            .checked_dependency_resolution(&operation)
            .await
            .context("dependency recovery is blocked; reservation retained")?;
        self.commit_dependency_resolution(&record, &graph, &resolved)
            .await?;
        Ok(resolved)
    }

    async fn checked_dependency_resolution(
        &self,
        operation: &DependencyOperation,
    ) -> anyhow::Result<(WorkflowGraph, DependencyOperation)> {
        tokio::time::timeout(RECOVERY_TIMEOUT, async {
            let graph = self.pending_dependency_graph(operation).await?;
            if operation.kind == super::DependencyOperationKind::Delete {
                self.verify_deletion_allocation(operation).await?;
            }
            let native = jj::operation_id(&operation.repository_path).await?;
            if native != operation.native_operation {
                // Uses the normal completion rule, including the exact operation
                // parent, native ancestry, subtree identities, and stable head.
                let (graph, mut resolved) = self.verify_dependency_operation(operation).await?;
                resolved.detail = format!(
                    "Recovered an applied native dependency edit without rerunning jj. {}",
                    resolved.detail
                );
                return Ok((graph, resolved));
            }
            let revisions = self.current_graph_revisions(&graph).await?;
            verify_reserved_dependencies(operation, &graph, &revisions)?;
            let source = revisions.get(&operation.node_id).context("source association missing")?;
            anyhow::ensure!(*source == operation.source, "source association or revision changed");
            let mut actual = jj::descendants(&operation.repository_path, source).await?;
            let mut expected = operation.before.clone();
            actual.sort_by(|a, b| a.commit_id.cmp(&b.commit_id));
            expected.sort_by(|a, b| a.commit_id.cmp(&b.commit_id));
            anyhow::ensure!(actual == expected, "native subtree no longer matches the reserved input");
            verify_graph_ancestry(&operation.repository_path, &graph, &revisions).await?;
            anyhow::ensure!(
                jj::operation_id(&operation.repository_path).await? == native,
                "native repository changed during recovery checks"
            );
            let mut resolved = operation.clone();
            resolved.state = DependencyOperationState::NotApplied;
            resolved.detail = "Recovered: native dependency edit was not applied. Saved dependencies, files, and the existing undo boundary are unchanged.".to_owned();
            Ok((graph, resolved))
        }).await.context("dependency recovery checks timed out; reservation retained")?
    }
}
