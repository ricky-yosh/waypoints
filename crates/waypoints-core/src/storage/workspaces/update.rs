//! Explicit stale-checkout updates, guarded by the saved allocation's readiness.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context as _;

use super::{Storage, StorageError};
use crate::jj;
use crate::workspace::{
    WorkspaceAssociation, WorkspaceState, WorkspaceUpdate, WorkspaceUpdateOutcome,
};

impl Storage {
    /// Explicitly updates a stale dedicated checkout with native jj preservation semantics.
    ///
    /// Fresh checkouts are a readonly no-op, even with dirty files. Before native
    /// mutation the allocation is durably marked incomplete. Only verified completion
    /// restores readiness; failures require inspection and explicit verification,
    /// never blind replay. No session barrier is removed.
    ///
    /// # Errors
    ///
    /// Refuses absent/incomplete/mismatched/shared workspaces, unsafe native state,
    /// active/interrupted writers and unresolved operations. Native diagnostics,
    /// timeout, postflight or storage failures may follow successful file changes.
    /// Stop old writers and inspect files/native jj before manual reconciliation
    /// and `verify_node_workspace`; this is not generic workspace repair.
    pub async fn update_stale_node_workspace(
        &mut self,
        workflow_id: i64,
        node_id: i64,
    ) -> Result<WorkspaceUpdate, StorageError> {
        self.ensure_dependency_operations_clear().await?;
        self.node(workflow_id, node_id).await?;
        let saved = self
            .saved_workspace(node_id)
            .await?
            .ok_or(super::super::StorageFailure::WorkspaceNotReady { node_id })
            .context("node has no saved workspace allocation")?;
        if !saved.ready {
            return Err(anyhow::Error::new(super::super::StorageFailure::WorkspaceNotReady { node_id })
                .context("workspace allocation is incomplete; inspect files and native jj, then explicitly verify a reconciled checkout; update-stale will not be replayed").into());
        }
        let association = &saved.association;
        let before = tokio::time::timeout(Duration::from_secs(40), async {
            self.ensure_workspace_update_idle().await?;
            let operation = jj::operation_id(&association.repository_path).await?;
            self.check_workspace_update_identity(workflow_id, node_id, association)
                .await?;
            let checked_out = jj::checked_out_revision(&association.directory).await?;
            // An older commit is expected, but a different change is an unfinished
            // allocation or identity drift, not a stale checkout we may update.
            anyhow::ensure!(
                checked_out.change_id == association.change_id,
                "physical checkout belongs to another change; no update attempted"
            );
            let current = jj::current_revision(&association.directory).await?;
            jj::ensure_mutable(&association.directory, &current).await?;
            anyhow::ensure!(
                jj::operation_id(&association.repository_path).await? == operation,
                "native repository changed during update preflight; no update attempted"
            );
            Ok::<_, anyhow::Error>((operation, checked_out.commit_id != current.commit_id))
        })
        .await
        .context("workspace update preflight timed out; no update attempted")??;

        if !before.1 {
            return Ok(WorkspaceUpdate {
                outcome: WorkspaceUpdateOutcome::AlreadyFresh,
                workspace: self.node_workspace(workflow_id, node_id).await?,
            });
        }
        let connection = self
            .database
            .connect()
            .context("connecting to workspace update storage")?;
        let changed = connection
            .execute(
                "UPDATE node_workspaces SET ready = 0 WHERE node_id = ?1 AND ready = 1",
                [node_id],
            )
            .await
            .context("reserving explicit workspace update")?;
        if changed != 1 {
            return Err(anyhow::anyhow!("workspace readiness changed before update").into());
        }
        tokio::time::timeout(Duration::from_secs(70), async {
            anyhow::ensure!(jj::operation_id(&association.repository_path).await? == before.0,
                "native repository changed before update");
            jj::update_stale_workspace(association).await?;
            self.check_workspace_update_identity(workflow_id, node_id, association).await?;
            let current = jj::current_revision(&association.directory).await?;
            anyhow::ensure!(jj::checked_out_revision(&association.directory).await? == current,
                "native update did not finish a fresh checkout");
            // Verification remains the single rule for acknowledging readiness.
            Ok::<_, anyhow::Error>(self.verify_node_workspace(workflow_id, node_id).await?)
        }).await.context("workspace update timed out; allocation remains incomplete; stop old writers and inspect before explicit verification; do not replay")?
            .map(|workspace| WorkspaceUpdate { outcome: WorkspaceUpdateOutcome::Updated, workspace })
            .map_err(|error| anyhow::anyhow!("workspace update outcome requires inspection; allocation remains incomplete; do not replay; stop old writers, inspect native jj/files and explicitly verify only after reconciliation: {error:#}").into())
    }

    async fn ensure_workspace_update_idle(&self) -> anyhow::Result<()> {
        let connection = self.database.connect()?;
        let mut rows = connection
            .query(
                "SELECT id FROM agent_runs WHERE active = 1
             UNION ALL SELECT node_id FROM node_change_creations
             UNION ALL SELECT node_id FROM node_workspaces WHERE ready = 0 LIMIT 1",
                (),
            )
            .await?;
        anyhow::ensure!(
            rows.next().await?.is_none(),
            "workspace update requires no active/interrupted runs, uncertain change creation, or incomplete workspace allocations in this database"
        );
        Ok(())
    }

    async fn check_workspace_update_identity(
        &self,
        workflow_id: i64,
        node_id: i64,
        saved: &WorkspaceAssociation,
    ) -> anyhow::Result<()> {
        let observed = self.observe_workspace(workflow_id, node_id, saved).await?;
        anyhow::ensure!(
            matches!(observed, WorkspaceState::Ready { .. }),
            "saved workspace identity is not ready: {observed:?}"
        );
        anyhow::ensure!(
            saved.directory != saved.repository_path,
            "update requires a dedicated workspace, not the original checkout"
        );
        anyhow::ensure!(
            repository_store(&saved.directory)? == repository_store(&saved.repository_path)?,
            "saved workspace points to a different native repository"
        );
        let registry = jj::workspaces(&saved.repository_path).await?;
        anyhow::ensure!(
            registry
                .iter()
                .filter(|item| item.change_id == saved.change_id)
                .all(|item| item.name == saved.name
                    && item.directory.as_ref() == Some(&saved.directory)),
            "another workspace edits this change; update refused"
        );
        let connection = self.database.connect()?;
        let mut rows = connection.query(
            "SELECT node_id FROM node_changes WHERE repository_path = ?1 AND change_id = ?2 AND node_id != ?3
             AND node_id NOT IN (SELECT node_id FROM deleted_nodes) LIMIT 1",
            turso::params![saved.repository_path.to_str(), saved.change_id.as_str(), node_id],
        ).await?;
        anyhow::ensure!(
            rows.next().await?.is_none(),
            "another node owns this workspace's change"
        );
        Ok(())
    }
}

// jj 0.46 local workspaces use a repository directory or a UTF-8 pointer relative
// to .jj. Resolve it without trusting registry paths copied from another repository.
fn repository_store(workspace: &Path) -> anyhow::Result<PathBuf> {
    let path = workspace.join(".jj/repo");
    let metadata = std::fs::symlink_metadata(&path)?;
    let target = if metadata.is_dir() {
        path
    } else {
        anyhow::ensure!(metadata.is_file(), "unsupported jj repository pointer");
        let mut pointer = String::new();
        std::fs::File::open(&path)?
            .take(4097)
            .read_to_string(&mut pointer)?;
        anyhow::ensure!(
            !pointer.is_empty() && pointer.len() <= 4096,
            "unsupported jj repository pointer"
        );
        workspace.join(".jj").join(pointer)
    };
    target
        .canonicalize()
        .context("resolving native jj repository identity")
}
