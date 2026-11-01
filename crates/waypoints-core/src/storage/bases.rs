//! Save exact workflow starting revisions without editing native state.

use std::path::Path;
use std::time::Duration;

use anyhow::Context;

use super::{
    Storage, StorageError, Workflow, canonical_project, decode_workflow, validate_workflow_name,
};
use crate::jj;
use crate::workflow::{BaseSource, WorkflowBase};

// Two bounded native queries, within the ordinary five-second client deadline.
const BASE_QUERY_TIMEOUT: Duration = Duration::from_secs(4);

impl Storage {
    pub(super) async fn initialize_bases(&self) -> anyhow::Result<()> {
        self.database
            .connect()?
            .execute(
                "CREATE TABLE IF NOT EXISTS workflow_bases (
                workflow_id INTEGER PRIMARY KEY REFERENCES workflows(id),
                record TEXT NOT NULL
            )",
                (),
            )
            .await
            .context("initializing workflow bases")?;
        Ok(())
    }

    /// Creates a workflow with an explicit, read-only resolved native starting point.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid project/name, missing or ambiguous native selection,
    /// unavailable jj, native timeout, or storage failure. Rejected creation saves nothing.
    pub async fn create_workflow_from(
        &mut self,
        project: &Path,
        name: &str,
        source: &BaseSource,
    ) -> Result<Workflow, StorageError> {
        validate_workflow_name(name)?;
        let project = canonical_project(project)?;
        let base = resolve(&project, source).await?;
        Ok(self
            .create_workflow_record(&project, name, Some(base))
            .await?)
    }

    pub(super) async fn create_workflow_record(
        &self,
        project: &Path,
        name: &str,
        base: Option<WorkflowBase>,
    ) -> anyhow::Result<Workflow> {
        validate_workflow_name(name)?;
        let project = canonical_project(project)?;
        let text = project.to_str().context("project path must be UTF-8")?;
        let mut connection = self.database.connect()?;
        let tx = connection.transaction().await?;
        let result = async {
            tx.execute(
                "INSERT INTO workflows (project_path, name) VALUES (?1, ?2)",
                [text, name],
            )
            .await
            .context("creating workflow")?;
            let id = tx.last_insert_rowid();
            if let Some(base) = &base {
                tx.execute(
                    "INSERT INTO workflow_bases (workflow_id, record) VALUES (?1, ?2)",
                    turso::params![id, serde_json::to_string(base)?],
                )
                .await
                .context("saving workflow base")?;
            }
            Ok(Workflow {
                id,
                project_path: project.clone(),
                name: name.to_owned(),
                base,
            })
        }
        .await;
        match result {
            Ok(workflow) => {
                tx.commit().await.context("committing workflow and base")?;
                Ok(workflow)
            }
            Err(error) => {
                tx.rollback()
                    .await
                    .context("rolling back workflow and base")?;
                Err(error)
            }
        }
    }

    /// Reads saved workflow metadata and base without accessing jj.
    ///
    /// # Errors
    ///
    /// Returns an error for a missing workflow or unreadable saved data.
    pub async fn workflow(&self, workflow_id: i64) -> Result<Workflow, StorageError> {
        Ok(async {
            let connection = self.database.connect()?;
            let mut rows = connection
                .query(
                    "SELECT w.id, w.project_path, w.name, b.record FROM workflows w
                 LEFT JOIN workflow_bases b ON b.workflow_id = w.id WHERE w.id = ?1",
                    [workflow_id],
                )
                .await?;
            let row = rows
                .next()
                .await?
                .with_context(|| format!("workflow {workflow_id} not found"))?;
            decode_workflow(&row)
        }
        .await?)
    }

    /// Selects an initial base for a legacy/unbased workflow, without rewriting its nodes.
    ///
    /// This is a one-time selection, not an update-base or graph-adoption operation.
    ///
    /// # Errors
    ///
    /// Returns an error if a base is already saved, the workflow is missing, selection
    /// fails, or storage fails. Existing prompts, edges, code, and history stay unchanged.
    pub async fn set_workflow_base(
        &mut self,
        workflow_id: i64,
        source: &BaseSource,
    ) -> Result<Workflow, StorageError> {
        let mut workflow = self.workflow(workflow_id).await?;
        if workflow.base.is_some() {
            return Err(anyhow::anyhow!(
                "workflow already has a base; changing it requires a separate explicit update operation"
            ).into());
        }
        let base = resolve(&workflow.project_path, source).await?;
        self.database
            .connect()
            .map_err(anyhow::Error::from)?
            .execute(
                "INSERT INTO workflow_bases (workflow_id, record) VALUES (?1, ?2)",
                turso::params![
                    workflow_id,
                    serde_json::to_string(&base).map_err(anyhow::Error::from)?
                ],
            )
            .await
            .context("saving initial workflow base")?;
        workflow.base = Some(base);
        Ok(workflow)
    }
}

async fn resolve(project: &Path, source: &BaseSource) -> anyhow::Result<WorkflowBase> {
    tokio::time::timeout(BASE_QUERY_TIMEOUT, async {
        let repository_path = jj::repository_root(project).await?;
        let revision = jj::resolve_workflow_base(&repository_path, source).await?;
        Ok(WorkflowBase {
            repository_path,
            source: source.clone(),
            revision,
        })
    })
    .await
    .context("workflow base selection timed out")?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_write_failure_rolls_back_new_workflow() -> anyhow::Result<()> {
        tokio::runtime::Builder::new_current_thread()
            .build()?
            .block_on(async {
                let directory = tempfile::tempdir()?;
                let storage = Storage::open(&directory.path().join("workflow.db")).await?;
                let connection = storage.database.connect()?;
                connection.execute("DROP TABLE workflow_bases", ()).await?;
                connection
                    .execute(
                        "CREATE TABLE workflow_bases (
                    workflow_id INTEGER PRIMARY KEY,
                    record TEXT NOT NULL CHECK (record = 'injected failure'))",
                        (),
                    )
                    .await?;
                let base = WorkflowBase {
                    repository_path: directory.path().to_owned(),
                    source: BaseSource::Bookmark {
                        name: "main".to_owned(),
                    },
                    revision: jj::Revision {
                        change_id: "k".repeat(32),
                        commit_id: "a".repeat(40),
                        description: "Baseline".to_owned(),
                        conflicted: false,
                    },
                };
                assert!(
                    storage
                        .create_workflow_record(directory.path(), "Must roll back", Some(base))
                        .await
                        .is_err()
                );
                assert!(storage.list_workflows(directory.path()).await?.is_empty());
                Ok(())
            })
    }
}
