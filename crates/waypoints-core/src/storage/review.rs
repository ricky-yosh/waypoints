use std::time::Duration;

use anyhow::Context;
use tokio::time::timeout;

use super::{Storage, StorageError};
use crate::jj::{self, ChangeState, RevisionReview};
use crate::run::SnapshotState;

// Keep the complete multi-query read within the ordinary five-second RPC deadline.
const REVIEW_TIMEOUT: Duration = Duration::from_secs(4);

impl Storage {
    /// Reviews a node's unique current recorded revision without snapshotting dirty files.
    ///
    /// # Errors
    ///
    /// Refuses unassociated, missing, or divergent changes, invalid membership,
    /// unavailable repositories, oversized output, and native/storage failures.
    pub async fn review_node(
        &self,
        workflow_id: i64,
        node_id: i64,
    ) -> Result<RevisionReview, StorageError> {
        Ok(timeout(REVIEW_TIMEOUT, async {
            let current = self.refresh_node(workflow_id, node_id).await?;
            let ChangeState::Current { revision } = current.state else {
                anyhow::bail!("review requires one current, associated native revision");
            };
            let association = current.association.context("missing native association")?;
            jj::review(&association.repository_path, revision).await
        })
        .await
        .context("revision review timed out")??)
    }

    /// Reviews a run's exact saved snapshot, never substituting the node's current revision.
    ///
    /// Historical and deleted-node runs remain readable through their saved project.
    /// This reads repository objects, not the run's potentially missing workspace.
    ///
    /// # Errors
    ///
    /// Refuses runs without a saved snapshot, unavailable or collected commits,
    /// changed revision identity, oversized output, and native/storage failures.
    pub async fn review_run(&self, run_id: i64) -> Result<RevisionReview, StorageError> {
        Ok(timeout(REVIEW_TIMEOUT, async {
            let run = self.run(run_id).await?;
            let SnapshotState::Saved { revision } = run.snapshot else {
                anyhow::bail!("run has no saved native snapshot to review");
            };
            let project = self.project_for_node(run.workflow_id, run.node_id).await?;
            let repository = jj::repository_root(&project).await?;
            let exact = jj::resolve_base(&repository, &revision.commit_id).await?;
            anyhow::ensure!(
                exact == revision,
                "saved snapshot identity does not match repository"
            );
            jj::review(&repository, exact).await
        })
        .await
        .context("revision review timed out")??)
    }
}
