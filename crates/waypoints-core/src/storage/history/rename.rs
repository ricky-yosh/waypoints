//! Undoable workflow naming; never changes graph, native, workspace, or run state.

use anyhow::Context;
use turso::transaction::TransactionBehavior;

use super::super::{Storage, StorageError, Workflow, decode_workflow, validate_workflow_name};
use super::{Edit, append, finish};

impl Storage {
    /// Renames a workflow as one project-level metadata history action.
    ///
    /// Names are preserved verbatim and need not be unique. An identical name is
    /// a no-op that preserves redo. Only the name is changed, including on replay.
    ///
    /// # Errors
    ///
    /// Returns an error for a blank name, missing workflow, unresolved dependency
    /// operation, unreadable workflow, or database failure. Rejected edits leave
    /// the name and history unchanged.
    pub async fn rename_workflow(
        &mut self,
        workflow_id: i64,
        name: &str,
    ) -> Result<Workflow, StorageError> {
        validate_workflow_name(name)?;
        self.ensure_dependency_operations_clear().await?;
        let mut connection = self
            .database
            .connect()
            .context("connecting to workflow database")?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .await
            .context("starting workflow rename transaction")?;
        let result = async {
            let mut rows = tx
                .query(
                    "SELECT w.id, w.project_path, w.name, b.record,
                 EXISTS (SELECT 1 FROM workflow_archives a WHERE a.workflow_id = w.id)
                 FROM workflows w
                 LEFT JOIN workflow_bases b ON b.workflow_id = w.id WHERE w.id = ?1",
                    [workflow_id],
                )
                .await?;
            let row = rows
                .next()
                .await?
                .with_context(|| format!("workflow {workflow_id} not found"))?;
            let mut workflow = decode_workflow(&row)?;
            drop(rows);
            if workflow.name != name {
                replace_name(&tx, workflow_id, &workflow.name, name).await?;
                append(
                    &tx,
                    &workflow.project_path,
                    &Edit::RenameWorkflow {
                        workflow_id,
                        before: workflow.name,
                        after: name.to_owned(),
                    },
                )
                .await?;
                workflow.name = name.to_owned();
            }
            Ok(workflow)
        }
        .await;
        Ok(finish(tx, result).await?)
    }
}

pub(super) async fn replace_name(
    tx: &turso::Connection,
    workflow_id: i64,
    expected: &str,
    replacement: &str,
) -> anyhow::Result<()> {
    validate_workflow_name(replacement)?;
    let changed = tx
        .execute(
            "UPDATE workflows SET name = ?3 WHERE id = ?1 AND name = ?2",
            turso::params![workflow_id, expected, replacement],
        )
        .await?;
    anyhow::ensure!(
        changed == 1,
        "cannot rename workflow: saved name changed or workflow missing"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(test: impl std::future::Future<Output = anyhow::Result<()>>) -> anyhow::Result<()> {
        tokio::runtime::Builder::new_current_thread()
            .build()?
            .block_on(test)
    }

    #[test]
    fn failed_history_and_cursor_writes_roll_back_names_and_redo() -> anyhow::Result<()> {
        run(async {
            let directory = tempfile::tempdir()?;
            let project = directory.path();
            let mut storage = Storage::open(&project.join("workflow.db")).await?;
            let workflow = storage.create_workflow(project, "Original").await?;
            storage.rename_workflow(workflow.id, "First").await?;
            let first = storage
                .history_status(project)
                .await?
                .undo
                .expect("first rename");
            storage.rename_workflow(workflow.id, "Second").await?;
            let second = storage
                .history_status(project)
                .await?
                .undo
                .expect("second rename");
            let pending = storage.undo(project, second.id).await?;
            let saved = storage.workflow(workflow.id).await?;
            let connection = storage.database.connect()?;
            // Fail after the name update and redo-tail deletion, not before the
            // transaction begins. The first edit keeps version 4 occupied.
            connection
                .execute(
                    &format!(
                        "CREATE UNIQUE INDEX fail_rename ON workflow_edits(version) WHERE id <> {}",
                        second.id
                    ),
                    (),
                )
                .await?;
            assert!(
                storage
                    .rename_workflow(workflow.id, "Must roll back")
                    .await
                    .is_err()
            );
            assert_eq!(storage.workflow(workflow.id).await?, saved);
            assert_eq!(storage.history_status(project).await?, pending);
            connection.execute("DROP INDEX fail_rename", ()).await?;
            connection
                .execute("DROP TABLE workflow_history_heads", ())
                .await?;
            connection
                .execute(
                    "CREATE TABLE workflow_history_heads (
                    project_path TEXT PRIMARY KEY, cursor INTEGER NOT NULL CHECK(cursor = 1))",
                    (),
                )
                .await?;
            connection
                .execute(
                    "INSERT INTO workflow_history_heads VALUES (?1, 1)",
                    [pending.project_path.to_str()],
                )
                .await?;
            for (undo, id) in [(true, first.id), (false, second.id)] {
                let result = if undo {
                    storage.undo(project, id).await
                } else {
                    storage.redo(project, id).await
                };
                assert!(result.is_err(), "cursor failure must reject replay");
                assert_eq!(storage.workflow(workflow.id).await?, saved);
                assert_eq!(storage.history_status(project).await?, pending);
            }
            Ok(())
        })
    }

    #[test]
    fn replay_refuses_changed_names_without_moving_history() -> anyhow::Result<()> {
        run(async {
            let directory = tempfile::tempdir()?;
            let project = directory.path();
            let mut storage = Storage::open(&project.join("workflow.db")).await?;
            let workflow = storage.create_workflow(project, "Original").await?;
            storage.rename_workflow(workflow.id, "Renamed").await?;
            let pending = storage.history_status(project).await?;
            let edit = pending.undo.as_ref().expect("rename").id;
            let connection = storage.database.connect()?;
            let mut rows = connection
                .query("SELECT version FROM workflow_edits WHERE id = ?1", [edit])
                .await?;
            assert_eq!(rows.next().await?.expect("saved rename").get::<i64>(0)?, 4);
            drop(rows);
            connection
                .execute("UPDATE workflows SET name = 'External'", ())
                .await?;
            assert!(storage.undo(project, edit).await.is_err());
            assert_eq!(storage.workflow(workflow.id).await?.name, "External");
            assert_eq!(storage.history_status(project).await?, pending);
            connection
                .execute("UPDATE workflows SET name = 'Renamed'", ())
                .await?;
            let pending = storage.undo(project, edit).await?;
            connection
                .execute("UPDATE workflows SET name = 'External again'", ())
                .await?;
            assert!(storage.redo(project, edit).await.is_err());
            assert_eq!(storage.workflow(workflow.id).await?.name, "External again");
            assert_eq!(storage.history_status(project).await?, pending);
            Ok(())
        })
    }
}
