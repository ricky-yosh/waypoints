//! Durable same-change tree restoration; agent conversations are never rewound.

use std::time::Duration;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use turso::transaction::TransactionBehavior;

use super::{Storage, StorageError};
use crate::jj::{self, Revision};
use crate::run::{RecoveryConfirmation, Run, SnapshotState};
use crate::workspace::{WorkspaceAssociation, WorkspaceState};

mod downstream;
mod impact;
mod schema;
#[cfg(test)]
mod tests;

pub use downstream::{RestoreAffectedNode, RestoreDownstream, RestoreRebaseConfirmation};
pub use impact::{
    RestoreImpact, RestoreImpactAllocation, RestoreImpactChange, RestoreImpactCheckout,
    RestoreImpactSession, RestoreImpactTask, RestoreImpactWorkspace,
};

// Leave time for transport and journal writes within the 120-second RPC deadline.
const RESTORE_TIMEOUT: Duration = Duration::from_secs(110);

/// Whether restoration finished or requires explicit native-state reconciliation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RestoreState {
    /// Reserved before any snapshot/restore; never automatically retried.
    Unresolved,
    /// Selected code was restored and its checkout verified.
    Completed,
    /// Caller reconciled the checkout; this does not claim the restore succeeded.
    Recovered,
}

/// Exact saved inputs and checkpoints for restoring code within one node/change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RestoreOperation {
    /// Database-local operation identifier.
    pub id: i64,
    /// Selected saved run, or none when a native commit was selected directly.
    pub run_id: Option<i64>,
    /// Owning workflow.
    pub workflow_id: i64,
    /// Unchanged owning node.
    pub node_id: i64,
    /// Workspace whose files may be changed.
    pub workspace: WorkspaceAssociation,
    /// Selected historical revision; never rewritten in the run record.
    pub source: Revision,
    /// Recorded current revision before snapshotting local edits.
    pub before: Revision,
    /// Exact preserved revision including jj-tracked local edits, when checkpointed.
    pub preserved: Option<Revision>,
    /// Verified result, or current observation after explicit recovery.
    pub after: Option<Revision>,
    /// Native operation before any mutation.
    pub native_operation: String,
    /// Sessions from runs at or below this ID cannot continue after restoration.
    pub session_barrier: i64,
    /// Completion or uncertainty, independent of any agent run outcome.
    pub state: RestoreState,
    /// Checked downstream scope and checkpoints; absent on legacy leaf restores.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downstream: Option<RestoreDownstream>,
}

impl Storage {
    pub(super) async fn initialize_restores(&self) -> anyhow::Result<()> {
        schema::initialize(&self.database).await
    }

    pub(super) async fn ensure_restores_clear(&self) -> anyhow::Result<()> {
        let connection = self.database.connect()?;
        let mut rows = connection
            .query(
                "SELECT restore_operations.id, restore_operations.node_id, nodes.workflow_id
                 FROM restore_operations LEFT JOIN nodes ON nodes.id = restore_operations.node_id
                 WHERE restore_operations.active = 1",
                (),
            )
            .await?;
        if let Some(row) = rows.next().await? {
            let id: i64 = row.get(0)?;
            let node_id: i64 = row.get(1)?;
            let workflow_id = row.get::<Option<i64>>(2)?
                .context("unresolved restore references a missing node; inspect the journal before further work")?;
            anyhow::bail!(
                "restore operation {id} is unresolved; inspect node restores --workflow {workflow_id} --node {node_id} and reconcile native jj state before explicit recovery; do not retry"
            );
        }
        Ok(())
    }

    pub(super) async fn ensure_session_after_restore(&self, run: &Run) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.session_was_restored(run).await?,
            "code restoration invalidated this session for continuation; explicitly start a fresh run"
        );
        Ok(())
    }

    pub(super) async fn session_was_restored(&self, run: &Run) -> anyhow::Result<bool> {
        let connection = self.database.connect()?;
        let mut rows = connection
            .query(
                "SELECT id FROM restore_operations WHERE session_barrier >= ?2
             AND (node_id = ?1 OR EXISTS (
               SELECT 1 FROM json_each(record, '$.downstream.nodes')
               WHERE json_extract(value, '$.node_id') = ?1)) LIMIT 1",
                turso::params![run.node_id, run.id],
            )
            .await?;
        Ok(rows.next().await?.is_some())
    }

    /// Restores a successful run's saved code into the same node's current change.
    ///
    /// First snapshots jj-tracked local edits and journals their exact revision.
    /// Uses native `jj restore`, retaining current change identity, description and
    /// parents, not `jj edit` on a hidden commit. All pre-existing sessions on this
    /// node are barred from continuation; explicitly start a fresh run afterwards.
    ///
    /// # Errors
    ///
    /// Refuses active/interrupted runs, any saved or native descendants, changed
    /// parents, missing snapshots, stale/shared workspaces and immutable changes.
    /// An uncertain mutation retains a durable global guard; never blindly retry.
    pub async fn restore_run(&mut self, run_id: i64) -> Result<RestoreOperation, StorageError> {
        let run = self.run(run_id).await?;
        let SnapshotState::Saved { revision } = &run.snapshot else {
            return Err(anyhow::anyhow!("run has no saved revision to restore").into());
        };
        self.restore_selected(
            run.workflow_id,
            run.node_id,
            &revision.commit_id,
            Some(&run),
        )
        .await
    }

    /// Restores an exact native revision within the same node/change, without requiring a run.
    ///
    /// Accepts a full lowercase commit ID for this change, including manual edits
    /// and preserved checkpoints. Uses the same checkpoint, session barrier,
    /// leaf/parent checks and durable recovery as saved-run restoration.
    ///
    /// # Errors
    ///
    /// Refuses invalid or missing commits, another change's revision, unsafe
    /// workspace/ancestry/run state and native/storage failures. An uncertain
    /// mutation remains reserved; never blindly repeat the request.
    pub async fn restore_node_revision(
        &mut self,
        workflow_id: i64,
        node_id: i64,
        commit_id: &str,
    ) -> Result<RestoreOperation, StorageError> {
        jj::validate_commit_id(commit_id)?;
        self.restore_selected(workflow_id, node_id, commit_id, None)
            .await
    }

    async fn restore_selected(
        &mut self,
        workflow_id: i64,
        node_id: i64,
        commit_id: &str,
        run: Option<&Run>,
    ) -> Result<RestoreOperation, StorageError> {
        Ok(tokio::time::timeout(RESTORE_TIMEOUT, async {
            self.ensure_dependency_operations_clear().await?;
            self.ensure_no_restore_writers().await?;
            self.node(workflow_id, node_id).await?;
            self.require_ready_workspace(node_id).await?;
            let verified = self.verify_node_workspace(workflow_id, node_id).await?;
            let WorkspaceState::Ready { revision: before } = verified.state else {
                anyhow::bail!("restore requires a ready workspace");
            };
            let workspace = verified.workspace.context("node has no workspace")?;
            let native_operation = jj::operation_id(&workspace.repository_path).await?;
            let source = jj::resolve_base(&workspace.repository_path, commit_id).await?;
            anyhow::ensure!(
                source.change_id == workspace.change_id,
                "selected revision belongs to another change; cross-change restoration is not supported"
            );
            if let Some(run) = run {
                anyhow::ensure!(
                    workspace.change_id == run.change_id && workspace.directory == run.directory,
                    "saved run no longer matches this node's workspace/change"
                );
                anyhow::ensure!(
                    run.snapshot == SnapshotState::Saved { revision: source.clone() },
                    "saved revision identity changed"
                );
            }
            self.check_restore_leaf(workflow_id, node_id, &workspace, &before).await?;
            anyhow::ensure!(
                jj::parent_commit_ids(&workspace.repository_path, &source.commit_id).await?
                    == jj::parent_commit_ids(&workspace.repository_path, &before.commit_id).await?,
                "saved revision has different parents; restoring across ancestry changes is not supported"
            );
            jj::ensure_mutable(&workspace.repository_path, &before).await?;
            anyhow::ensure!(
                jj::operation_id(&workspace.repository_path).await? == native_operation,
                "native repository changed during restore preflight; no mutation started"
            );
            let mut operation = RestoreOperation {
                id: 0, run_id: run.map(|run| run.id), workflow_id, node_id,
                workspace, source, before, preserved: None, after: None,
                native_operation, session_barrier: 0, state: RestoreState::Unresolved,
                downstream: None,
            };
            self.reserve_restore(&mut operation).await?;
            let result = async {
                anyhow::ensure!(
                    jj::operation_id(&operation.workspace.repository_path).await?
                        == operation.native_operation,
                    "native repository changed after reservation"
                );
                let preserved = jj::snapshot_workspace(&operation.workspace, &operation.before).await?;
                operation.preserved = Some(preserved.clone());
                self.save_restore(&operation).await?;
                self.check_restore_leaf(
                    operation.workflow_id, operation.node_id, &operation.workspace, &preserved,
                ).await?;
                operation.after = Some(jj::restore_workspace(
                    &operation.workspace, &operation.source, &preserved,
                ).await?);
                operation.state = RestoreState::Completed;
                self.save_restore(&operation).await?;
                Ok::<_, anyhow::Error>(())
            }.await;
            result.with_context(|| format!(
                "restore operation {} may be incomplete; inspect node restores --workflow {} --node {}; no automatic retry or rollback",
                operation.id, operation.workflow_id, operation.node_id,
            ))?;
            Ok::<_, anyhow::Error>(operation)
        }).await.context(
            "restore timed out; inspect restore status and native jj state before further work",
        )??)
    }

    /// Lists restoration attempts for a selected run without changing native state.
    ///
    /// # Errors
    ///
    /// Returns an error for a missing run or unreadable journal.
    pub async fn run_restores(&self, run_id: i64) -> Result<Vec<RestoreOperation>, StorageError> {
        self.run(run_id).await?;
        Ok(async {
            let connection = self.database.connect()?;
            let mut rows = connection
                .query(
                    "SELECT record FROM restore_operations WHERE run_id = ?1 ORDER BY id",
                    [run_id],
                )
                .await?;
            let mut operations = Vec::new();
            while let Some(row) = rows.next().await? {
                operations.push(serde_json::from_str(&row.get::<String>(0)?)?);
            }
            Ok::<_, anyhow::Error>(operations)
        }
        .await?)
    }

    /// Lists every saved-run and exact-commit restoration attempt for a node.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid membership or an unreadable journal.
    pub async fn node_restores(
        &self,
        workflow_id: i64,
        node_id: i64,
    ) -> Result<Vec<RestoreOperation>, StorageError> {
        self.node(workflow_id, node_id).await?;
        Ok(async {
            let connection = self.database.connect()?;
            let mut rows = connection
                .query(
                    "SELECT record FROM restore_operations WHERE node_id = ?1 OR EXISTS (
                       SELECT 1 FROM json_each(record, '$.downstream.nodes')
                       WHERE json_extract(value, '$.node_id') = ?1) ORDER BY id",
                    [node_id],
                )
                .await?;
            let mut operations = Vec::new();
            while let Some(row) = rows.next().await? {
                operations.push(serde_json::from_str(&row.get::<String>(0)?)?);
            }
            Ok::<_, anyhow::Error>(operations)
        }
        .await?)
    }

    /// Reads a restoration journal without modifying native or working-copy state.
    ///
    /// # Errors
    ///
    /// Returns an error for an absent operation or unreadable journal.
    pub async fn restore_operation(&self, id: i64) -> Result<RestoreOperation, StorageError> {
        Ok(async {
            let connection = self.database.connect()?;
            let mut rows = connection
                .query("SELECT record FROM restore_operations WHERE id = ?1", [id])
                .await?;
            let row = rows.next().await?.context("restore operation not found")?;
            Ok::<_, anyhow::Error>(serde_json::from_str(&row.get::<String>(0)?)?)
        }
        .await?)
    }

    /// Releases an uncertain restore after explicit native checkout reconciliation.
    ///
    /// Never retries restoration, snapshots files, or attributes current code to a
    /// past run. Caller must stop native writers and manually reconcile any stale
    /// checkout first. The old-session barrier remains in force.
    ///
    /// # Errors
    ///
    /// Refuses absent confirmation, active runs, changed identity/parents,
    /// descendants, stale checkouts or native/storage failures.
    pub async fn recover_restore(
        &mut self,
        id: i64,
        confirmation: Option<RecoveryConfirmation>,
    ) -> Result<RestoreOperation, StorageError> {
        Ok(tokio::time::timeout(RESTORE_TIMEOUT, async {
            anyhow::ensure!(
                confirmation == Some(RecoveryConfirmation::ProcessesStopped),
                "restore recovery requires --confirm-processes-stopped"
            );
            let mut operation = self.restore_operation(id).await?;
            if operation.state != RestoreState::Unresolved {
                return Ok(operation);
            }
            self.ensure_no_restore_writers().await?;
            if operation.downstream.is_some() {
                return self.recover_downstream_restore(operation).await;
            }
            let verified = self
                .verify_node_workspace(operation.workflow_id, operation.node_id)
                .await?;
            anyhow::ensure!(
                verified.workspace.as_ref() == Some(&operation.workspace),
                "restore workspace allocation changed"
            );
            let WorkspaceState::Ready { revision } = verified.state else {
                anyhow::bail!("restore workspace is not ready; reconcile native jj state first");
            };
            self.check_restore_leaf(
                operation.workflow_id,
                operation.node_id,
                &operation.workspace,
                &revision,
            )
            .await?;
            anyhow::ensure!(
                jj::parent_commit_ids(&operation.workspace.repository_path, &revision.commit_id)
                    .await?
                    == jj::parent_commit_ids(
                        &operation.workspace.repository_path,
                        &operation.before.commit_id
                    )
                    .await?,
                "restore recovery refuses changed parents"
            );
            operation.after = Some(revision);
            operation.state = RestoreState::Recovered;
            self.save_restore(&operation).await?;
            Ok::<_, anyhow::Error>(operation)
        })
        .await
        .context("restore recovery timed out; reservation retained")??)
    }

    async fn ensure_no_restore_writers(&self) -> anyhow::Result<()> {
        let connection = self.database.connect()?;
        let mut rows = connection
            .query("SELECT id FROM agent_runs WHERE active = 1 LIMIT 1", ())
            .await?;
        anyhow::ensure!(
            rows.next().await?.is_none(),
            "stop or recover all active/interrupted agent runs before restoring code"
        );
        Ok(())
    }

    async fn check_restore_leaf(
        &self,
        workflow_id: i64,
        node_id: i64,
        workspace: &WorkspaceAssociation,
        revision: &Revision,
    ) -> anyhow::Result<()> {
        let graph = self.workflow_graph(workflow_id).await?;
        anyhow::ensure!(
            !graph
                .connections
                .iter()
                .any(|edge| edge.source_id == node_id),
            "restore currently requires a node without downstream tasks"
        );
        anyhow::ensure!(
            jj::descendants(&workspace.repository_path, revision).await? == [revision.clone()],
            "restore currently requires a change without native descendants"
        );
        let workspaces = jj::workspaces(&workspace.repository_path).await?;
        anyhow::ensure!(
            workspaces
                .iter()
                .filter(|item| item.change_id == workspace.change_id)
                .all(|item| item.name == workspace.name
                    && item.directory.as_ref() == Some(&workspace.directory)),
            "another workspace edits this change; restore refused"
        );
        Ok(())
    }

    async fn reserve_restore(&self, operation: &mut RestoreOperation) -> anyhow::Result<()> {
        let mut connection = self.database.connect()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .await?;
        let result = async {
            if let Some(scope) = &operation.downstream {
                anyhow::ensure!(
                    super::dependencies::load_graph(&tx, operation.workflow_id).await? == scope.graph,
                    "saved graph changed before restore reservation"
                );
            }
            let mut rows = tx.query("SELECT COALESCE(MAX(id), 0) FROM agent_runs", ()).await?;
            operation.session_barrier = rows.next().await?.context("missing run maximum")?.get(0)?;
            let changed = tx.execute(
                "INSERT INTO restore_operations(run_id, node_id, active, session_barrier, record)
                 SELECT ?1, ?2, 1, ?3, ?4 WHERE NOT EXISTS (SELECT 1 FROM agent_runs WHERE active = 1)
                 AND (?5 IS NULL OR (SELECT COALESCE(MAX(id), 0) FROM restore_operations) = ?5)",
                turso::params![operation.run_id, operation.node_id, operation.session_barrier,
                    serde_json::to_string(operation)?,
                    operation.downstream.as_ref().map(|scope| scope.preview_restore_watermark)],
            ).await?;
            anyhow::ensure!(changed == 1, "a run became active or the confirmed journal changed before restore reservation");
            operation.id = tx.last_insert_rowid();
            tx.execute("UPDATE restore_operations SET record = ?2 WHERE id = ?1",
                turso::params![operation.id, serde_json::to_string(operation)?]).await?;
            Ok::<_, anyhow::Error>(())
        }.await;
        match result {
            Ok(()) => tx.commit().await?,
            Err(error) => {
                tx.rollback().await?;
                return Err(error);
            }
        }
        Ok(())
    }

    async fn save_restore(&self, operation: &RestoreOperation) -> anyhow::Result<()> {
        let connection = self.database.connect()?;
        let changed = connection.execute(
            "UPDATE restore_operations SET active = ?2, record = ?3 WHERE id = ?1 AND active = 1",
            turso::params![operation.id, i64::from(operation.state == RestoreState::Unresolved),
                serde_json::to_string(operation)?],
        ).await?;
        anyhow::ensure!(
            changed == 1,
            "restore reservation changed; inspect stored operation"
        );
        Ok(())
    }
}
