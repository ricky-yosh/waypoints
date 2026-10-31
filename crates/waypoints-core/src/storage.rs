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

/// A prompt-bearing node belonging to one workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Node {
    /// The database-local node identifier.
    pub id: i64,
    /// The workflow containing this node.
    pub workflow_id: i64,
    /// The user-provided prompt, preserved verbatim.
    pub prompt: String,
}

/// A directed edge between nodes in the same workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Connection {
    /// The workflow containing both endpoints.
    pub workflow_id: i64,
    /// The upstream node identifier.
    pub source_id: i64,
    /// The downstream node identifier.
    pub target_id: i64,
}

/// A workflow and its directed acyclic graph, ordered by node and endpoint identifiers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct WorkflowGraph {
    /// The saved workflow metadata.
    pub workflow: Workflow,
    /// Nodes in ascending identifier order.
    pub nodes: Vec<Node>,
    /// Connections in ascending source, then target identifier order.
    pub connections: Vec<Connection>,
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
        connection
            .execute(
                "CREATE TABLE IF NOT EXISTS nodes (
                    id INTEGER PRIMARY KEY,
                    workflow_id INTEGER NOT NULL REFERENCES workflows(id),
                    prompt TEXT NOT NULL,
                    UNIQUE (workflow_id, id)
                )",
                (),
            )
            .await
            .context("initializing node schema")?;
        connection
            .execute(
                "CREATE TABLE IF NOT EXISTS connections (
                    workflow_id INTEGER NOT NULL,
                    source_id INTEGER NOT NULL,
                    target_id INTEGER NOT NULL,
                    PRIMARY KEY (workflow_id, source_id, target_id),
                    CHECK (source_id <> target_id),
                    FOREIGN KEY (workflow_id, source_id) REFERENCES nodes(workflow_id, id),
                    FOREIGN KEY (workflow_id, target_id) REFERENCES nodes(workflow_id, id)
                )",
                (),
            )
            .await
            .context("initializing connection schema")?;
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

    /// Creates a node, preserving all nonblank prompt text verbatim.
    ///
    /// # Errors
    ///
    /// Returns an error for a blank prompt, a missing workflow, or a database failure.
    pub async fn create_node(
        &mut self,
        workflow_id: i64,
        prompt: &str,
    ) -> Result<Node, StorageError> {
        validate_prompt(prompt)?;
        let connection = self
            .database
            .connect()
            .context("connecting to workflow database")?;
        let changed = connection
            .execute(
                "INSERT INTO nodes (workflow_id, prompt)
                 SELECT id, ?2 FROM workflows WHERE id = ?1",
                turso::params![workflow_id, prompt],
            )
            .await
            .context("creating node")?;
        if changed != 1 {
            return Err(anyhow::anyhow!("workflow {workflow_id} not found").into());
        }
        Ok(Node {
            id: connection.last_insert_rowid(),
            workflow_id,
            prompt: prompt.to_owned(),
        })
    }

    /// Replaces a node's prompt without changing its identifier or connections.
    ///
    /// # Errors
    ///
    /// Returns an error for a blank prompt, a missing workflow or node, a node in
    /// another workflow, or a database failure.
    pub async fn update_node(
        &mut self,
        workflow_id: i64,
        node_id: i64,
        prompt: &str,
    ) -> Result<Node, StorageError> {
        validate_prompt(prompt)?;
        let connection = self
            .database
            .connect()
            .context("connecting to workflow database")?;
        let changed = connection
            .execute(
                "UPDATE nodes SET prompt = ?3 WHERE workflow_id = ?1 AND id = ?2
                 AND EXISTS (SELECT 1 FROM workflows WHERE id = ?1)",
                turso::params![workflow_id, node_id, prompt],
            )
            .await
            .context("updating node")?;
        if changed != 1 {
            return Err(
                anyhow::anyhow!("node {node_id} not found in workflow {workflow_id}").into(),
            );
        }
        Ok(Node {
            id: node_id,
            workflow_id,
            prompt: prompt.to_owned(),
        })
    }

    /// Loads a node only when it belongs to the requested workflow.
    ///
    /// # Errors
    ///
    /// Returns an error for a missing workflow or node, a node in another workflow,
    /// or a database failure.
    pub async fn node(&self, workflow_id: i64, node_id: i64) -> Result<Node, StorageError> {
        let connection = self
            .database
            .connect()
            .context("connecting to workflow database")?;
        let mut rows = connection
            .query(
                "SELECT id, workflow_id, prompt FROM nodes
                 WHERE workflow_id = ?1 AND id = ?2
                 AND EXISTS (SELECT 1 FROM workflows WHERE id = ?1)",
                [workflow_id, node_id],
            )
            .await
            .context("querying node")?;
        let row = rows
            .next()
            .await
            .context("reading node row")?
            .with_context(|| format!("node {node_id} not found in workflow {workflow_id}"))?;
        Ok(decode_node(&row)?)
    }

    /// Adds a directed connection while preserving an acyclic workflow graph.
    ///
    /// # Errors
    ///
    /// Returns an error for missing or foreign endpoints, a missing workflow,
    /// a self-link, duplicate link, cycle, or database failure.
    pub async fn connect_nodes(
        &mut self,
        workflow_id: i64,
        source_id: i64,
        target_id: i64,
    ) -> Result<Connection, StorageError> {
        let connection = self
            .database
            .connect()
            .context("connecting to workflow database")?;
        // Validation and insertion share one statement, including across storage handles.
        // UNION deduplicates reachable nodes in a DAG with converging paths.
        let changed = connection
            .execute(
                "WITH RECURSIVE reachable(id) AS (
                    SELECT ?3
                    UNION
                    SELECT c.target_id FROM connections c JOIN reachable r ON c.source_id = r.id
                    WHERE c.workflow_id = ?1
                 )
                 INSERT INTO connections (workflow_id, source_id, target_id)
                 SELECT ?1, ?2, ?3
                 WHERE ?2 <> ?3
                   AND EXISTS (SELECT 1 FROM workflows WHERE id = ?1)
                   AND EXISTS (SELECT 1 FROM nodes WHERE workflow_id = ?1 AND id = ?2)
                   AND EXISTS (SELECT 1 FROM nodes WHERE workflow_id = ?1 AND id = ?3)
                   AND NOT EXISTS (
                       SELECT 1 FROM connections
                       WHERE workflow_id = ?1 AND source_id = ?2 AND target_id = ?3
                   )
                   AND NOT EXISTS (SELECT 1 FROM reachable WHERE id = ?2)",
                [workflow_id, source_id, target_id],
            )
            .await
            .context("connecting nodes")?;
        if changed != 1 {
            return Err(anyhow::anyhow!(
                "invalid connection {source_id} -> {target_id} in workflow {workflow_id}: \
                 missing workflow or endpoints, self-link, duplicate link, or cycle"
            )
            .into());
        }
        Ok(Connection {
            workflow_id,
            source_id,
            target_id,
        })
    }

    /// Loads a workflow graph with deterministic node and connection ordering.
    ///
    /// # Errors
    ///
    /// Returns an error for a missing workflow or a database query or decoding failure.
    pub async fn workflow_graph(&self, workflow_id: i64) -> Result<WorkflowGraph, StorageError> {
        let connection = self
            .database
            .connect()
            .context("connecting to workflow database")?;
        let mut rows = connection
            .query(
                "SELECT id, project_path, name FROM workflows WHERE id = ?1",
                [workflow_id],
            )
            .await
            .context("querying workflow")?;
        let row = rows
            .next()
            .await
            .context("reading workflow row")?
            .with_context(|| format!("workflow {workflow_id} not found"))?;
        let workflow = Workflow {
            id: row.get(0).context("reading workflow identifier")?,
            project_path: PathBuf::from(
                row.get::<String>(1)
                    .context("reading workflow project path")?,
            ),
            name: row.get(2).context("reading workflow name")?,
        };
        let mut rows = connection
            .query(
                "SELECT id, workflow_id, prompt FROM nodes WHERE workflow_id = ?1 ORDER BY id",
                [workflow_id],
            )
            .await
            .context("listing nodes")?;
        let mut nodes = Vec::new();
        while let Some(row) = rows.next().await.context("reading node row")? {
            nodes.push(decode_node(&row)?);
        }
        let mut rows = connection
            .query(
                "SELECT workflow_id, source_id, target_id FROM connections
                 WHERE workflow_id = ?1 ORDER BY source_id, target_id",
                [workflow_id],
            )
            .await
            .context("listing connections")?;
        let mut connections = Vec::new();
        while let Some(row) = rows.next().await.context("reading connection row")? {
            connections.push(Connection {
                workflow_id: row
                    .get(0)
                    .context("reading connection workflow identifier")?,
                source_id: row.get(1).context("reading connection source identifier")?,
                target_id: row.get(2).context("reading connection target identifier")?,
            });
        }
        Ok(WorkflowGraph {
            workflow,
            nodes,
            connections,
        })
    }
}

fn validate_prompt(prompt: &str) -> anyhow::Result<()> {
    anyhow::ensure!(!prompt.trim().is_empty(), "node prompt must not be blank");
    Ok(())
}

fn decode_node(row: &turso::Row) -> anyhow::Result<Node> {
    Ok(Node {
        id: row.get(0).context("reading node identifier")?,
        workflow_id: row.get(1).context("reading node workflow identifier")?,
        prompt: row.get(2).context("reading node prompt")?,
    })
}

fn canonical_project(path: &Path) -> anyhow::Result<PathBuf> {
    let canonical = fs::canonicalize(path)
        .with_context(|| format!("resolving project directory {}", path.display()))?;
    anyhow::ensure!(canonical.is_dir(), "project path must be a directory");
    Ok(canonical)
}
