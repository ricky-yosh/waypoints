//! File-backed workflow storage, independent of the GUI and transport.

use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::jj::{self, ChangeAssociation, ChangeState};
use crate::workflow::{GraphConnectivity, NodeParent, WorkflowBase};

mod archive;
mod bases;
mod dependencies;
mod history;
mod reparent;
mod restore;
mod review;
mod revisions;
mod runs;
mod upstream;
mod workspaces;

pub use history::{HistoryEntry, HistoryStatus};
pub use reparent::{
    DeletionWorkspace, DependencyOperation, DependencyOperationKind, DependencyOperationState,
    DependencyRecovery, DependencyRecoveryAssessment, UndoBoundaryAcknowledgement,
};
pub use restore::{
    RestoreAffectedNode, RestoreDownstream, RestoreImpact, RestoreImpactAllocation,
    RestoreImpactChange, RestoreImpactCheckout, RestoreImpactSession, RestoreImpactTask,
    RestoreImpactWorkspace, RestoreOperation, RestoreRebaseConfirmation, RestoreState,
};
pub use revisions::{NodeRevisionEntry, NodeRevisionPage};

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
    /// Hidden from the active list; all saved data and operations remain available.
    #[serde(default)]
    pub archived: bool,
}

/// Selects which saved workflows appear in a project list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum WorkflowFilter {
    /// Workflows not archived; the default list.
    #[default]
    Active,
    /// Only archived workflows.
    Archived,
    /// Both active and archived workflows.
    All,
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
    /// Node IDs explicitly attached directly to the workflow base, in ascending order.
    #[serde(default)]
    pub base_nodes: Vec<i64>,
    /// Saved reachability, distinct from native ancestry and working-copy freshness.
    #[serde(default)]
    pub connectivity: GraphConnectivity,
    /// Interrupted native edit affecting this graph; saved edges may not match jj.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unresolved_dependency_operation: Option<i64>,
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

/// Explicit actionable storage refusals; other failures retain their diagnostic causes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum StorageFailure {
    /// No workflow with this database-local identifier exists.
    MissingWorkflow { workflow_id: i64 },
    /// No visible node with this identifier belongs to the requested workflow.
    MissingNode { workflow_id: i64, node_id: i64 },
    /// No saved run with this database-local identifier exists.
    MissingRun { run_id: i64 },
    /// A run still reserves this node, including interrupted runs.
    NodeBusy { node_id: i64, run_id: i64 },
    /// A required workspace is absent or its allocation is not acknowledged ready.
    WorkspaceNotReady { node_id: i64 },
}

impl fmt::Display for StorageFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingWorkflow { workflow_id } => {
                write!(formatter, "workflow {workflow_id} not found")
            }
            Self::MissingNode {
                workflow_id,
                node_id,
            } => write!(
                formatter,
                "node {node_id} not found in workflow {workflow_id}"
            ),
            Self::MissingRun { .. } => formatter.write_str("run not found"),
            Self::NodeBusy { run_id, .. } => write!(
                formatter,
                "node is reserved by run {run_id}; inspect its status first"
            ),
            Self::WorkspaceNotReady { .. } => formatter.write_str("workspace is not ready"),
        }
    }
}

impl Error for StorageFailure {}

impl StorageError {
    /// Returns an explicit refusal, even when wrapped in diagnostic context.
    pub fn failure(&self) -> Option<&StorageFailure> {
        self.inner.downcast_ref().or_else(|| {
            self.inner
                .downcast_ref::<StorageError>()
                .and_then(Self::failure)
        })
    }
}

impl fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.fmt(formatter)
    }
}

impl Error for StorageError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.inner.source()
    }
}

impl From<anyhow::Error> for StorageError {
    fn from(inner: anyhow::Error) -> Self {
        Self { inner }
    }
}

impl From<StorageFailure> for StorageError {
    fn from(failure: StorageFailure) -> Self {
        Self {
            inner: failure.into(),
        }
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
        storage.initialize_archives().await?;
        storage.initialize_runs().await?;
        storage.initialize_history().await?;
        storage.initialize_bases().await?;
        storage.initialize_dependencies().await?;
        storage.initialize_dependency_operations().await?;
        storage.initialize_restores().await?;
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

    /// Lists a project's active workflows in ascending identifier order.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid or non-UTF-8 project path, or if querying
    /// or decoding saved workflows fails.
    pub async fn list_workflows(&self, project_path: &Path) -> Result<Vec<Workflow>, StorageError> {
        self.list_workflows_filtered(project_path, WorkflowFilter::Active)
            .await
    }

    /// Lists active, archived, or all workflows in ascending identifier order.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid project path or unreadable saved data.
    pub async fn list_workflows_filtered(
        &self,
        project_path: &Path,
        filter: WorkflowFilter,
    ) -> Result<Vec<Workflow>, StorageError> {
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
                "SELECT w.id, w.project_path, w.name, b.record,
                 EXISTS (SELECT 1 FROM workflow_archives a WHERE a.workflow_id = w.id)
                 FROM workflows w
                 LEFT JOIN workflow_bases b ON b.workflow_id = w.id
                 WHERE w.project_path = ?1 ORDER BY w.id",
                [project],
            )
            .await
            .context("listing workflows")?;
        let mut workflows = Vec::new();
        while let Some(row) = rows.next().await.context("reading workflow row")? {
            let workflow = decode_workflow(&row)?;
            if match filter {
                WorkflowFilter::Active => !workflow.archived,
                WorkflowFilter::Archived => workflow.archived,
                WorkflowFilter::All => true,
            } {
                workflows.push(workflow);
            }
        }
        Ok(workflows)
    }

    /// Creates a node, preserving all nonblank prompt text verbatim.
    ///
    /// # Errors
    ///
    /// Returns an error for a blank prompt, missing workflow, a based workflow
    /// requiring an explicit dependency, or a database failure.
    pub async fn create_node(
        &mut self,
        workflow_id: i64,
        prompt: &str,
    ) -> Result<Node, StorageError> {
        validate_prompt(prompt)?;
        Ok(self
            .create_node_with_history(workflow_id, prompt, &[])
            .await?)
    }

    /// Creates a node and its explicit dependency as one undoable metadata action.
    ///
    /// # Errors
    ///
    /// Returns an error for an unbased/unadopted workflow, invalid parent, blank
    /// prompt, or storage failure. Does not create or rebase native code.
    pub async fn create_node_from(
        &mut self,
        workflow_id: i64,
        prompt: &str,
        parent: &NodeParent,
    ) -> Result<Node, StorageError> {
        self.create_node_from_parents(workflow_id, prompt, std::slice::from_ref(parent))
            .await
    }

    /// Creates a node with all selected upstream dependencies as one undoable action.
    ///
    /// Select one or more distinct nodes from this rooted based workflow, or
    /// the workflow base alone. This saves metadata without creating native code.
    ///
    /// # Errors
    ///
    /// Returns an error for empty/duplicate/cross-workflow parents, an unrooted
    /// workflow, a base mixed with nodes, a blank prompt, or storage failure.
    pub async fn create_node_from_parents(
        &mut self,
        workflow_id: i64,
        prompt: &str,
        parents: &[NodeParent],
    ) -> Result<Node, StorageError> {
        validate_prompt(prompt)?;
        if parents.is_empty() {
            return Err(anyhow::anyhow!("select at least one node dependency").into());
        }
        Ok(self
            .create_node_with_history(workflow_id, prompt, parents)
            .await?)
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
                 AND NOT EXISTS (SELECT 1 FROM deleted_nodes WHERE node_id = nodes.id)
                 AND EXISTS (SELECT 1 FROM workflows WHERE id = ?1)",
                [workflow_id, node_id],
            )
            .await
            .context("querying node")?;
        let row =
            rows.next()
                .await
                .context("reading node row")?
                .ok_or(StorageFailure::MissingNode {
                    workflow_id,
                    node_id,
                })?;
        Ok(decode_node(&row)?)
    }

    /// Associates a node with one existing, unambiguous native jj change.
    ///
    /// A unique change-ID prefix is resolved to its full ID before saving.
    /// Repeating this command explicitly replaces a previous association.
    /// This does not snapshot the working copy or change jj history. Based workflows
    /// require matching native parents and exclusive node ownership of the change.
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
        tokio::time::timeout(
            std::time::Duration::from_secs(4),
            self.validate_dependency_association(workflow_id, node_id, &repository_path, &revision),
        )
        .await
        .context("node dependency validation timed out")??;
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
    /// Accepts a change ID, commit ID, unique prefix, or `root()`. In a based workflow
    /// the resolved revision must match the node's saved dependency. The working copy
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
        self.create_node_change_selected(workflow_id, node_id, Some(base))
            .await
    }

    /// Creates a node's native change on its saved dependency, not an arbitrary base.
    ///
    /// # Errors
    ///
    /// Returns an error for legacy/unrooted graphs, unavailable/busy upstream work,
    /// duplicate native parents, or native/storage failures. Multiple dependencies
    /// produce a native merge; conflicts are recorded, not automatically resolved.
    pub async fn create_node_change_from_dependency(
        &mut self,
        workflow_id: i64,
        node_id: i64,
    ) -> Result<NodeChange, StorageError> {
        self.create_node_change_selected(workflow_id, node_id, None)
            .await
    }

    async fn create_node_change_selected(
        &mut self,
        workflow_id: i64,
        node_id: i64,
        base: Option<&str>,
    ) -> Result<NodeChange, StorageError> {
        self.ensure_dependency_operations_clear().await?;
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
                let dependency = self.dependency_revisions(workflow_id, node_id).await?;
                let selected = match base {
                    Some(base) => Some(jj::resolve_base(&root, base).await?),
                    None => None,
                };
                let parents = match dependency {
                    Some((dependency_root, parents)) => {
                        anyhow::ensure!(root == dependency_root,
                            "saved dependencies must belong to the same repository");
                        if let Some(selected) = selected {
                            anyhow::ensure!(parents.len() == 1 && selected.commit_id == parents[0].commit_id,
                                "requested native base differs from the node's saved dependency");
                        }
                        parents
                    }
                    None => vec![selected.context("legacy workflows require an explicit --base for native change creation")?],
                };
                let unique: std::collections::HashSet<_> = parents.iter().map(|parent| &parent.commit_id).collect();
                anyhow::ensure!(unique.len() == parents.len(), "saved dependencies resolve to duplicate native revisions");
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
                let revision = jj::create_change_from_parents(&root, &parents).await
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
        let Some(association) = self.node_change_association(node_id).await? else {
            return Ok(NodeChange {
                node,
                association: None,
                state: ChangeState::Unassociated,
            });
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

    async fn node_change_association(
        &self,
        node_id: i64,
    ) -> anyhow::Result<Option<ChangeAssociation>> {
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
            return Ok(None);
        };
        let association = ChangeAssociation {
            repository_path: PathBuf::from(
                row.get::<String>(0).context("reading jj repository path")?,
            ),
            change_id: row.get(1).context("reading jj change ID")?,
        };
        Ok(Some(association))
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

    /// Adds a metadata-only connection in a legacy unbased workflow, preserving a DAG.
    ///
    /// # Errors
    ///
    /// Returns an error for missing or foreign endpoints, a missing workflow,
    /// a self-link, duplicate link, cycle, a based workflow, or database failure.
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
        Ok(dependencies::load_graph(&connection, workflow_id).await?)
    }
}

fn validate_prompt(prompt: &str) -> anyhow::Result<()> {
    anyhow::ensure!(!prompt.trim().is_empty(), "node prompt must not be blank");
    Ok(())
}

fn validate_workflow_name(name: &str) -> anyhow::Result<()> {
    anyhow::ensure!(!name.trim().is_empty(), "workflow name must not be blank");
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
        archived: row
            .get::<i64>(4)
            .context("reading workflow archive state")?
            != 0,
    })
}

fn canonical_project(path: &Path) -> anyhow::Result<PathBuf> {
    let canonical = fs::canonicalize(path)
        .with_context(|| format!("resolving project directory {}", path.display()))?;
    anyhow::ensure!(canonical.is_dir(), "project path must be a directory");
    Ok(canonical)
}
