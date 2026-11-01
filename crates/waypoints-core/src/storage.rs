//! File-backed workflow storage, independent of the GUI and transport.

use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::jj::{self, ChangeAssociation, ChangeState};
use crate::workflow::WorkflowBase;

mod bases;
mod history;
mod runs;
mod workspaces;

pub use history::{HistoryEntry, HistoryStatus};

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
    /// Exact saved starting point, or none for an unbased/legacy workflow.
    #[serde(default)]
    pub base: Option<WorkflowBase>,
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

/// A saved node and its freshly queried native-change status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct NodeChange {
    /// The node, retained even when the associated change disappears.
    pub node: Node,
    /// The persisted reference, or none when no change has been selected.
    pub association: Option<ChangeAssociation>,
    /// Live native state; repository/query failures are errors, not missing changes.
    pub state: ChangeState,
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
        connection
            .execute(
                "CREATE TABLE IF NOT EXISTS node_changes (
                    node_id INTEGER PRIMARY KEY REFERENCES nodes(id),
                    repository_path TEXT NOT NULL,
                    change_id TEXT NOT NULL
                )",
                (),
            )
            .await
            .context("initializing node change schema")?;
        connection
            .execute(
                "CREATE TABLE IF NOT EXISTS node_change_creations (
                    node_id INTEGER PRIMARY KEY REFERENCES nodes(id)
                )",
                (),
            )
            .await
            .context("initializing change creation guards")?;
        connection
            .execute(
                "CREATE TABLE IF NOT EXISTS node_workspaces (
                    node_id INTEGER PRIMARY KEY REFERENCES nodes(id),
                    repository_path TEXT NOT NULL,
                    change_id TEXT NOT NULL,
                    name TEXT NOT NULL,
                    directory TEXT NOT NULL UNIQUE,
                    ready INTEGER NOT NULL DEFAULT 0 CHECK (ready IN (0, 1))
                )",
                (),
            )
            .await
            .context("initializing node workspace schema")?;
        let storage = Self { database };
        storage.initialize_runs().await?;
        storage.initialize_history().await?;
        storage.initialize_bases().await?;
        Ok(storage)
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
        Ok(self
            .create_workflow_record(project_path, name, None)
            .await?)
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
                "SELECT w.id, w.project_path, w.name, b.record FROM workflows w
                 LEFT JOIN workflow_bases b ON b.workflow_id = w.id
                 WHERE w.project_path = ?1 ORDER BY w.id",
                [project],
            )
            .await
            .context("listing workflows")?;
        let mut workflows = Vec::new();
        while let Some(row) = rows.next().await.context("reading workflow row")? {
            workflows.push(decode_workflow(&row)?);
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
        Ok(self.create_node_with_history(workflow_id, prompt).await?)
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
        Ok(self
            .update_node_with_history(workflow_id, node_id, prompt)
            .await?)
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

    /// Associates a node with one existing, unambiguous native jj change.
    ///
    /// A unique change-ID prefix is resolved to its full ID before saving.
    /// Repeating this command explicitly replaces a previous association.
    /// This does not snapshot the working copy or change jj history.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid node membership, IDs, missing/divergent changes,
    /// an unavailable jj repository/executable, or storage failures. Failed selection
    /// leaves any saved association unchanged.
    pub async fn associate_node(
        &mut self,
        workflow_id: i64,
        node_id: i64,
        change_id: &str,
    ) -> Result<NodeChange, StorageError> {
        let node = self.node(workflow_id, node_id).await?;
        self.ensure_node_idle(node_id).await?;
        jj::validate_change_id(change_id)?;
        let project = self.project_for_node(workflow_id, node_id).await?;
        let repository_path = jj::repository_root(&project).await?;
        let mut revisions = jj::revisions(&repository_path, change_id).await?;
        if revisions.len() != 1 {
            return Err(anyhow::anyhow!("select one existing, non-divergent jj change").into());
        }
        let revision = revisions.remove(0);
        let association = ChangeAssociation {
            repository_path,
            change_id: revision.change_id.clone(),
        };
        let connection = self
            .database
            .connect()
            .context("connecting to workflow database")?;
        connection
            .execute(
                "INSERT INTO node_changes (node_id, repository_path, change_id) VALUES (?1, ?2, ?3)
             ON CONFLICT(node_id) DO UPDATE SET
                 repository_path = excluded.repository_path, change_id = excluded.change_id",
                turso::params![
                    node_id,
                    association
                        .repository_path
                        .to_str()
                        .context("jj path must be UTF-8")?,
                    association.change_id.as_str()
                ],
            )
            .await
            .context("saving node change association")?;
        connection
            .execute(
                "DELETE FROM node_change_creations WHERE node_id = ?1",
                [node_id],
            )
            .await
            .context("clearing change creation guard")?;
        Ok(NodeChange {
            node,
            association: Some(association),
            state: ChangeState::Current { revision },
        })
    }

    /// Creates and associates an empty native change on an explicitly selected base.
    ///
    /// Accepts a change ID, commit ID, unique prefix, or `root()`. The working copy
    /// and prompt are unchanged. Existing associations, including missing ones, are
    /// refused. A durable guard prevents retries after uncertain native outcomes.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid membership/base, an existing association or
    /// creation guard, jj failures, timeouts, or database failures. After creation
    /// starts, failures require inspecting jj and explicitly associating the result;
    /// native history is never automatically rolled back.
    pub async fn create_node_change(
        &mut self,
        workflow_id: i64,
        node_id: i64,
        base: &str,
    ) -> Result<NodeChange, StorageError> {
        let node = self.node(workflow_id, node_id).await?;
        let connection = self
            .database
            .connect()
            .context("connecting to workflow database")?;
        let mut rows = connection
            .query(
                "SELECT node_id FROM node_changes WHERE node_id = ?1
             UNION ALL SELECT node_id FROM node_change_creations WHERE node_id = ?1",
                [node_id],
            )
            .await
            .context("checking node change creation eligibility")?;
        if rows
            .next()
            .await
            .context("reading creation eligibility")?
            .is_some()
        {
            return Err(anyhow::anyhow!(
                "node already has an association or an unfinished creation; inspect jj and explicitly associate the result, do not retry creation"
            ).into());
        }
        drop(rows);
        let project = self.project_for_node(workflow_id, node_id).await?;
        // Keep all native work within the client's five-second RPC deadline.
        let (repository_path, revision) = tokio::time::timeout(
            std::time::Duration::from_secs(4),
            async {
                let root = jj::repository_root(&project).await?;
                let base = jj::resolve_base(&root, base).await?;
                let reserved = connection.execute(
                    "INSERT INTO node_change_creations (node_id)
                     SELECT ?1 WHERE NOT EXISTS (SELECT 1 FROM node_changes WHERE node_id = ?1)",
                    [node_id],
                ).await.context("reserving node change creation")?;
                // A competing association must never turn this into an unguarded creation.
                anyhow::ensure!(
                    reserved == 1,
                    "node already has a change association"
                );
                let revision = jj::create_change(&root, &base).await
                    .context("native creation may have succeeded; inspect jj before explicitly associating the result")?;
                Ok::<_, anyhow::Error>((root, revision))
            },
        ).await.context("change creation timed out; inspect jj before retrying")??;
        let association = ChangeAssociation {
            repository_path,
            change_id: revision.change_id.clone(),
        };
        connection.execute(
            "INSERT INTO node_changes (node_id, repository_path, change_id) VALUES (?1, ?2, ?3)",
            turso::params![
                node_id,
                association.repository_path.to_str().context("jj path must be UTF-8")?,
                association.change_id.as_str()
            ],
        ).await.with_context(|| format!(
            "jj change {} was created, but saving failed; explicitly associate this change, do not create another",
            association.change_id
        ))?;
        connection
            .execute(
                "DELETE FROM node_change_creations WHERE node_id = ?1",
                [node_id],
            )
            .await
            .context("clearing completed creation guard")?;
        Ok(NodeChange {
            node,
            association: Some(association),
            state: ChangeState::Current { revision },
        })
    }

    /// Refreshes a saved association without mutating repository or workflow state.
    ///
    /// An absent visible change is reported as missing; multiple visible revisions
    /// are reported as divergent with two candidate revisions. Neither is guessed at
    /// or automatically repaired.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid membership, unavailable/moved repositories,
    /// jj failures or storage failures. These are never reported as missing changes.
    pub async fn refresh_node(
        &self,
        workflow_id: i64,
        node_id: i64,
    ) -> Result<NodeChange, StorageError> {
        let node = self.node(workflow_id, node_id).await?;
        let connection = self
            .database
            .connect()
            .context("connecting to workflow database")?;
        let mut rows = connection
            .query(
                "SELECT repository_path, change_id FROM node_changes WHERE node_id = ?1",
                [node_id],
            )
            .await
            .context("reading node change association")?;
        let Some(row) = rows.next().await.context("reading node change row")? else {
            return Ok(NodeChange {
                node,
                association: None,
                state: ChangeState::Unassociated,
            });
        };
        let association = ChangeAssociation {
            repository_path: PathBuf::from(
                row.get::<String>(0).context("reading jj repository path")?,
            ),
            change_id: row.get(1).context("reading jj change ID")?,
        };
        let project = self.project_for_node(workflow_id, node_id).await?;
        let root = jj::repository_root(&project).await?;
        if root != association.repository_path {
            return Err(anyhow::anyhow!(
                "project's jj workspace root changed; explicitly associate the node again"
            )
            .into());
        }
        let mut revisions = jj::revisions(&root, &association.change_id).await?;
        let state = match revisions.len() {
            0 => ChangeState::Missing,
            1 => ChangeState::Current {
                revision: revisions.remove(0),
            },
            _ => ChangeState::Divergent { revisions },
        };
        Ok(NodeChange {
            node,
            association: Some(association),
            state,
        })
    }

    async fn project_for_node(
        &self,
        workflow_id: i64,
        node_id: i64,
    ) -> Result<PathBuf, StorageError> {
        let connection = self
            .database
            .connect()
            .context("connecting to workflow database")?;
        let mut rows = connection
            .query(
                "SELECT w.project_path FROM workflows w JOIN nodes n ON n.workflow_id = w.id
             WHERE w.id = ?1 AND n.id = ?2",
                [workflow_id, node_id],
            )
            .await
            .context("reading node project")?;
        let row = rows
            .next()
            .await
            .context("reading node project row")?
            .context("node project not found")?;
        Ok(PathBuf::from(
            row.get::<String>(0).context("reading project path")?,
        ))
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
        Ok(self
            .connect_nodes_with_history(workflow_id, source_id, target_id)
            .await?)
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
                "SELECT w.id, w.project_path, w.name, b.record FROM workflows w
                 LEFT JOIN workflow_bases b ON b.workflow_id = w.id WHERE w.id = ?1",
                [workflow_id],
            )
            .await
            .context("querying workflow")?;
        let row = rows
            .next()
            .await
            .context("reading workflow row")?
            .with_context(|| format!("workflow {workflow_id} not found"))?;
        let workflow = decode_workflow(&row)?;
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

fn decode_workflow(row: &turso::Row) -> anyhow::Result<Workflow> {
    let record: Option<String> = row.get(3).context("reading workflow base")?;
    Ok(Workflow {
        id: row.get(0).context("reading workflow identifier")?,
        project_path: PathBuf::from(row.get::<String>(1).context("reading project path")?),
        name: row.get(2).context("reading workflow name")?,
        base: record
            .map(|record| serde_json::from_str(&record))
            .transpose()
            .context("decoding saved workflow base")?,
    })
}

fn canonical_project(path: &Path) -> anyhow::Result<PathBuf> {
    let canonical = fs::canonicalize(path)
        .with_context(|| format!("resolving project directory {}", path.display()))?;
    anyhow::ensure!(canonical.is_dir(), "project path must be a directory");
    Ok(canonical)
}
