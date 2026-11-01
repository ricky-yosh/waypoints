//! Explicit abandonment precedes deregistration; workspace directories are never modified.

use std::path::Path;
use std::time::Duration;

use anyhow::Context;

use super::super::{
    DeletionWorkspace, DependencyOperation, DependencyOperationKind, DependencyOperationState,
};
use crate::jj::{self, Revision};
use crate::run::RecoveryConfirmation;
use crate::storage::{Storage, StorageError};
use crate::workspace::{WorkspaceAssociation, WorkspaceState};

pub(super) async fn verify_allocation(
    connection: &turso::Connection,
    operation: &DependencyOperation,
) -> anyhow::Result<()> {
    let mut rows = connection.query(
        "SELECT repository_path, change_id, name, directory, ready FROM node_workspaces WHERE node_id = ?1",
        [operation.node_id],
    ).await?;
    match (rows.next().await?, &operation.deletion_workspace) {
        (None, None) => Ok(()),
        (Some(row), Some(expected)) => {
            let actual = WorkspaceAssociation {
                repository_path: row.get::<String>(0)?.into(),
                change_id: row.get(1)?,
                name: row.get(2)?,
                directory: row.get::<String>(3)?.into(),
            };
            anyhow::ensure!(
                actual == expected.workspace && row.get::<i64>(4)? == 1,
                "saved workspace allocation changed during deletion"
            );
            Ok(())
        }
        _ => anyhow::bail!("saved workspace allocation changed during deletion"),
    }
}

impl Storage {
    pub(in crate::storage::reparent) async fn verify_deletion_allocation(
        &self,
        operation: &DependencyOperation,
    ) -> anyhow::Result<()> {
        verify_allocation(&self.database.connect()?, operation).await
    }

    pub(in crate::storage::reparent) async fn deletion_workspace_preflight(
        &self,
        workflow_id: i64,
        node_id: i64,
        root: &Path,
        source: &Revision,
    ) -> anyhow::Result<Option<DeletionWorkspace>> {
        let observed = self.node_workspace(workflow_id, node_id).await?;
        let saved = observed.workspace;
        if let Some(workspace) = &saved {
            anyhow::ensure!(
                matches!(observed.state, WorkspaceState::Ready { .. }),
                "deletion requires a ready, matching saved workspace; inspect workspace status"
            );
            anyhow::ensure!(
                workspace.repository_path == root
                    && workspace.directory != root
                    && workspace.change_id == source.change_id,
                "deletion cannot deregister the project's repository workspace or a mismatched allocation"
            );
        }
        let registry = jj::workspaces(root).await?;
        for native in registry.iter().filter(|w| w.change_id == source.change_id) {
            anyhow::ensure!(
                saved.as_ref().is_some_and(|saved| native.name == saved.name
                    && native.directory.as_ref() == Some(&saved.directory)),
                "an unsaved or additional native workspace edits this change; reconcile explicitly"
            );
        }
        Ok(saved.map(|workspace| DeletionWorkspace {
            workspace,
            abandoned_operation: None,
        }))
    }

    pub(in crate::storage::reparent) async fn verify_forgotten_registry(
        &self,
        operation: &DependencyOperation,
    ) -> anyhow::Result<()> {
        self.verify_deletion_allocation(operation).await?;
        let registry = jj::workspaces(&operation.repository_path).await?;
        anyhow::ensure!(
            registry
                .iter()
                .all(|native| native.change_id != operation.source.change_id
                    && operation
                        .deletion_workspace
                        .as_ref()
                        .is_none_or(|saved| native.name != saved.workspace.name
                            && native.directory.as_ref() != Some(&saved.workspace.directory))),
            "workspace is still registered or another workspace edits the deleted change"
        );
        Ok(())
    }

    pub(in crate::storage::reparent) async fn verify_abandoned_deletion(
        &self,
        operation: &DependencyOperation,
    ) -> anyhow::Result<String> {
        tokio::time::timeout(Duration::from_secs(40), async {
            anyhow::ensure!(
                operation.kind == DependencyOperationKind::Delete
                    && operation.state == DependencyOperationState::Unresolved,
                "operation is not an unresolved deletion"
            );
            let workspace = operation
                .deletion_workspace
                .as_ref()
                .context("deletion has no saved workspace")?;
            let root = &operation.repository_path;
            let native = jj::operation_id(root).await?;
            anyhow::ensure!(
                workspace
                    .abandoned_operation
                    .as_ref()
                    .is_none_or(|saved| *saved == native),
                "repository is not at the exact abandonment checkpoint"
            );
            self.verify_deletion_result(operation, &operation.native_operation)
                .await?;
            self.verify_deletion_allocation(operation).await?;
            let registry = jj::workspaces(root).await?;
            let saved = registry
                .iter()
                .find(|entry| entry.name == workspace.workspace.name)
                .context("saved workspace is no longer registered")?;
            anyhow::ensure!(
                saved.directory.as_ref() == Some(&workspace.workspace.directory)
                    && registry
                        .iter()
                        .all(|entry| entry.change_id != operation.source.change_id),
                "workspace registry no longer matches the abandonment outcome"
            );
            // Abandonment assigns this workspace a replacement working-copy change.
            // Forget may automatically abandon that empty replacement, so it must not
            // be an associated node's change in any workflow.
            let connection = self.database.connect()?;
            let mut rows = connection.query(
                "SELECT node_id FROM node_changes WHERE repository_path = ?1 AND change_id = ?2",
                turso::params![root.to_str(), saved.change_id.clone()],
            ).await?;
            anyhow::ensure!(
                rows.next().await?.is_none(),
                "workspace replacement change belongs to a saved node; refusing deregistration"
            );
            anyhow::ensure!(
                jj::operation_id(root).await? == native,
                "repository changed during abandonment verification"
            );
            Ok(native)
        })
        .await
        .context("abandonment checkpoint verification timed out")?
    }

    pub(in crate::storage::reparent) async fn checkpoint_abandoned_deletion(
        &self,
        operation: &mut DependencyOperation,
    ) -> anyhow::Result<()> {
        let native = self.verify_abandoned_deletion(operation).await?;
        let (record, saved) = self.saved_dependency_operation(operation.id).await?;
        anyhow::ensure!(saved == *operation, "deletion reservation changed");
        let mut checkpoint = operation.clone();
        checkpoint
            .deletion_workspace
            .as_mut()
            .context("workspace intent missing")?
            .abandoned_operation = Some(native);
        checkpoint.detail = "Source change abandoned; saved workspace is still registered. Folder and files retained. Inspect operation recovery before explicitly resuming deregistration.".to_owned();
        let changed = self.database.connect()?.execute(
            "UPDATE dependency_operations SET record = ?2 WHERE id = ?1 AND active = 1 AND record = ?3",
            turso::params![operation.id, serde_json::to_string(&checkpoint)?, record],
        ).await?;
        anyhow::ensure!(
            changed == 1,
            "deletion reservation changed before checkpoint write"
        );
        *operation = checkpoint;
        Ok(())
    }

    pub(in crate::storage::reparent) async fn forget_deletion_workspace(
        &self,
        operation: &DependencyOperation,
    ) -> anyhow::Result<()> {
        let workspace = operation
            .deletion_workspace
            .as_ref()
            .context("workspace intent missing")?;
        let anchor = workspace
            .abandoned_operation
            .as_deref()
            .context("abandonment checkpoint missing")?;
        jj::forget_workspace(
            &operation.repository_path,
            &workspace.workspace.name,
            anchor,
        )
        .await
    }

    /// Continues only a verified abandoned deletion after explicit stopped-writer confirmation.
    ///
    /// Never repeats source abandonment. This explicitly authorizes remaining workspace deregistration,
    /// unlike metadata-only recovery. Directories, checkout metadata, and file contents are untouched.
    ///
    /// # Errors
    ///
    /// Refuses absent confirmation, ambiguous/intervening native changes, or storage/native failures.
    /// An uncertain outcome retains the durable reservation; inspect before taking further action.
    pub async fn resume_node_deletion(
        &mut self,
        id: i64,
        confirmation: Option<RecoveryConfirmation>,
    ) -> Result<DependencyOperation, StorageError> {
        Ok(async {
            anyhow::ensure!(confirmation == Some(RecoveryConfirmation::ProcessesStopped),
                "confirm old native processes and other writers stopped with --confirm-processes-stopped");
            let (_, mut operation) = self.saved_dependency_operation(id).await?;
            anyhow::ensure!(operation.kind == DependencyOperationKind::Delete
                && operation.deletion_workspace.is_some(), "operation is not a workspace-backed deletion");
            if operation.state != DependencyOperationState::Unresolved { return Ok(operation); }
            tokio::time::timeout(Duration::from_secs(70), async {
                self.checkpoint_abandoned_deletion(&mut operation).await?;
                self.forget_deletion_workspace(&operation).await?;
                self.finish_reparent(&mut operation).await
            }).await.context("resumed deletion timed out; reservation retained")??;
            Ok::<_, anyhow::Error>(operation)
        }.await?)
    }
}

#[cfg(test)]
mod tests;
