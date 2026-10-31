//! File-backed workflow storage, independent of the GUI and transport.

use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

/// A workflow saved for a canonical project directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Workflow {
    /// The database-local workflow identifier.
    pub id: i64,
    /// The canonical directory containing the project.
    pub project_path: PathBuf,
    /// The user-provided workflow name.
    pub name: String,
}

/// A failure to validate workflow input or access local storage.
#[derive(Debug)]
pub struct StorageError {
    inner: anyhow::Error,
}

impl fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.fmt(formatter)
    }
}

impl Error for StorageError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.inner.as_ref())
    }
}

impl From<anyhow::Error> for StorageError {
    fn from(inner: anyhow::Error) -> Self {
        Self { inner }
    }
}

/// A local workflow database owned by the backend.
#[derive(Debug)]
pub struct Storage {
    database: turso::Database,
}

impl Storage {
    /// Opens a database file, creating its initial schema when necessary.
    ///
    /// The parent directory must already exist. Non-UTF-8 paths are rejected
    /// because the database driver accepts UTF-8 filenames.
    ///
    /// # Errors
    ///
    /// Returns an error if the path is unsupported, opening fails, or schema
    /// initialization fails.
    pub async fn open(path: &Path) -> Result<Self, StorageError> {
        let filename = path.to_str().context("database path must be UTF-8")?;
        let database = turso::Builder::new_local(filename)
            .build()
            .await
            .with_context(|| format!("opening workflow database at {}", path.display()))?;
        let connection = database
            .connect()
            .context("connecting to workflow database")?;
        connection
            .execute(
                "CREATE TABLE IF NOT EXISTS workflows (
                    id INTEGER PRIMARY KEY,
                    project_path TEXT NOT NULL,
                    name TEXT NOT NULL CHECK (length(trim(name)) > 0)
                )",
                (),
            )
            .await
            .context("initializing workflow schema")?;
        Ok(Self { database })
    }

    /// Saves a named workflow for an existing project directory.
    ///
    /// The project path is canonicalized; the name is preserved verbatim.
    /// Multiple workflows may have the same name.
    ///
    /// # Errors
    ///
    /// Returns an error for a blank name, an invalid or non-UTF-8 project path,
    /// or a database failure.
    pub async fn create_workflow(
        &mut self,
        project_path: &Path,
        name: &str,
    ) -> Result<Workflow, StorageError> {
        if name.trim().is_empty() {
            return Err(anyhow::anyhow!("workflow name must not be blank").into());
        }
        let project_path = canonical_project(project_path)?;
        let project = project_path
            .to_str()
            .context("project path must be UTF-8")?;
        let connection = self
            .database
            .connect()
            .context("connecting to workflow database")?;
        connection
            .execute(
                "INSERT INTO workflows (project_path, name) VALUES (?1, ?2)",
                [project, name],
            )
            .await
            .context("creating workflow")?;
        Ok(Workflow {
            id: connection.last_insert_rowid(),
            project_path,
            name: name.to_owned(),
        })
    }

    /// Lists a project's saved workflows in ascending identifier order.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid or non-UTF-8 project path, or if querying
    /// or decoding saved workflows fails.
    pub async fn list_workflows(&self, project_path: &Path) -> Result<Vec<Workflow>, StorageError> {
        let project_path = canonical_project(project_path)?;
        let project = project_path
            .to_str()
            .context("project path must be UTF-8")?;
        let connection = self
            .database
            .connect()
            .context("connecting to workflow database")?;
        let mut rows = connection
            .query(
                "SELECT id, project_path, name FROM workflows WHERE project_path = ?1 ORDER BY id",
                [project],
            )
            .await
            .context("listing workflows")?;
        let mut workflows = Vec::new();
        while let Some(row) = rows.next().await.context("reading workflow row")? {
            workflows.push(Workflow {
                id: row.get(0).context("reading workflow identifier")?,
                project_path: PathBuf::from(
                    row.get::<String>(1)
                        .context("reading workflow project path")?,
                ),
                name: row.get(2).context("reading workflow name")?,
            });
        }
        Ok(workflows)
    }
}

fn canonical_project(path: &Path) -> anyhow::Result<PathBuf> {
    let canonical = fs::canonicalize(path)
        .with_context(|| format!("resolving project directory {}", path.display()))?;
    anyhow::ensure!(canonical.is_dir(), "project path must be a directory");
    Ok(canonical)
}
