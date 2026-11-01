//! Durable same-change tree restoration; agent conversations are never rewound.

use std::time::Duration;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use turso::transaction::TransactionBehavior;

use super::{Storage, StorageError};
use crate::jj::{self, Revision};
use crate::run::{RecoveryConfirmation, Run, SnapshotState};
use crate::workspace::{WorkspaceAssociation, WorkspaceState};

#[cfg(test)]
mod tests;

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
    /// Run whose saved code was selected.
    pub run_id: i64,
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
}

impl Storage {
    pub(super) async fn initialize_restores(&self) -> anyhow::Result<()> {
        let connection = self.database.connect()?;
        connection
            .execute(
                "CREATE TABLE IF NOT EXISTS restore_operations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                run_id INTEGER NOT NULL REFERENCES agent_runs(id),
                node_id INTEGER NOT NULL REFERENCES nodes(id),
                active INTEGER NOT NULL CHECK(active IN (0, 1)),
                session_barrier INTEGER NOT NULL,
                record TEXT NOT NULL
            )",
                (),
            )
            .await?;
        connection
            .execute(
                "CREATE UNIQUE INDEX IF NOT EXISTS one_restore_operation
             ON restore_operations(active) WHERE active = 1",
                (),
            )
            .await?;
        Ok(())
    }

    pub(super) async fn ensure_restores_clear(&self) -> anyhow::Result<()> {
        let connection = self.database.connect()?;
        let mut rows = connection
            .query(
                "SELECT id, run_id FROM restore_operations WHERE active = 1",
                (),
            )
            .await?;
        if let Some(row) = rows.next().await? {
            let id: i64 = row.get(0)?;
            let run_id: i64 = row.get(1)?;
            anyhow::bail!(
                "restore operation {id} is unresolved; inspect run restore-status --run {run_id}; reconcile native jj state and explicitly recover it; do not retry"
            );
        }
        Ok(())
    }

    pub(super) async fn ensure_session_after_restore(&self, run: &Run) -> anyhow::Result<()> {
        let connection = self.database.connect()?;
        let mut rows = connection.query(
            "SELECT id FROM restore_operations WHERE node_id = ?1 AND session_barrier >= ?2 LIMIT 1",
            turso::params![run.node_id, run.id],
        ).await?;
        anyhow::ensure!(
            rows.next().await?.is_none(),
            "code restoration invalidated this session for continuation; explicitly start a fresh run"
        );
        Ok(())
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
        Ok(tokio::time::timeout(RESTORE_TIMEOUT, async {
            self.ensure_dependency_operations_clear().await?;
            self.ensure_no_restore_writers().await?;
            let run = self.run(run_id).await?;
            let SnapshotState::Saved { revision: source } = &run.snapshot else {
                anyhow::bail!("run has no saved revision to restore");
            };
            self.node(run.workflow_id, run.node_id).await?;
            self.require_ready_workspace(run.node_id).await?;
            let verified = self.verify_node_workspace(run.workflow_id, run.node_id).await?;
            let WorkspaceState::Ready { revision: before } = verified.state else {
                anyhow::bail!("restore requires a ready workspace");
            };
            let workspace = verified.workspace.context("node has no workspace")?;
            anyhow::ensure!(
                workspace.change_id == run.change_id
                    && workspace.directory == run.directory
                    && source.change_id == workspace.change_id,
                "saved run no longer matches this node's workspace/change"
            );
            let native_operation = jj::operation_id(&workspace.repository_path).await?;
            let exact = jj::resolve_base(&workspace.repository_path, &source.commit_id).await?;
            anyhow::ensure!(exact == *source, "saved revision identity changed");
            self.check_restore_leaf(run.workflow_id, run.node_id, &workspace, &before).await?;
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
                id: 0, run_id, workflow_id: run.workflow_id, node_id: run.node_id,
                workspace, source: source.clone(), before, preserved: None, after: None,
                native_operation, session_barrier: 0, state: RestoreState::Unresolved,
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
                "restore operation {} may be incomplete; inspect run restore-status --run {}; no automatic retry or rollback",
                operation.id, operation.run_id,
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
            let mut rows = tx.query("SELECT COALESCE(MAX(id), 0) FROM agent_runs", ()).await?;
            operation.session_barrier = rows.next().await?.context("missing run maximum")?.get(0)?;
            let changed = tx.execute(
                "INSERT INTO restore_operations(run_id, node_id, active, session_barrier, record)
                 SELECT ?1, ?2, 1, ?3, ?4 WHERE NOT EXISTS (SELECT 1 FROM agent_runs WHERE active = 1)",
                turso::params![operation.run_id, operation.node_id, operation.session_barrier,
                    serde_json::to_string(operation)?],
            ).await?;
            anyhow::ensure!(changed == 1, "an agent run became active before restore");
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
