//! Reversible list visibility without changing workflow contents or native state.

use anyhow::Context;

use super::{Storage, StorageError, Workflow};

impl Storage {
    pub(super) async fn initialize_archives(&self) -> anyhow::Result<()> {
        self.database
            .connect()?
            .execute(
                "CREATE TABLE IF NOT EXISTS workflow_archives (
                    workflow_id INTEGER PRIMARY KEY REFERENCES workflows(id)
                )",
                (),
            )
            .await
            .context("initializing workflow archives")?;
        Ok(())
    }

    /// Hides a workflow from the active list without changing or locking its contents.
    ///
    /// Repeating this operation is a no-op. Runs are not stopped, and native
    /// changes, workspace files, and project undo/redo history are untouched.
    ///
    /// # Errors
    ///
    /// Returns an error for a missing workflow or storage failure.
    pub async fn archive_workflow(&mut self, workflow_id: i64) -> Result<Workflow, StorageError> {
        self.set_workflow_archived(workflow_id, true).await
    }

    /// Returns a workflow to the active list, preserving its identifier and contents.
    ///
    /// Repeating this operation is a no-op.
    ///
    /// # Errors
    ///
    /// Returns an error for a missing workflow or storage failure.
    pub async fn unarchive_workflow(&mut self, workflow_id: i64) -> Result<Workflow, StorageError> {
        self.set_workflow_archived(workflow_id, false).await
    }

    async fn set_workflow_archived(
        &mut self,
        workflow_id: i64,
        archived: bool,
    ) -> Result<Workflow, StorageError> {
        let mut workflow = self.workflow(workflow_id).await?;
        let connection = self
            .database
            .connect()
            .context("connecting to workflow database")?;
        let statement = if archived {
            "INSERT INTO workflow_archives (workflow_id) VALUES (?1) ON CONFLICT DO NOTHING"
        } else {
            "DELETE FROM workflow_archives WHERE workflow_id = ?1"
        };
        connection
            .execute(statement, [workflow_id])
            .await
            .context("saving workflow archive state")?;
        workflow.archived = archived;
        Ok(workflow)
    }
}
