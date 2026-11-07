//! Transactional workflow metadata history; never runs agents or native jj commands.

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use turso::transaction::{Transaction, TransactionBehavior};

use super::dependencies;
use super::{Connection, Node, Storage, StorageError, canonical_project, decode_node};
use crate::workflow::NodeParent;

#[cfg(test)]
#[path = "history/multiple_parents_tests.rs"]
mod multiple_parents_tests;
mod planning;
mod rename;

/// One pending undo or redo action, identified independently of its cursor position.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct HistoryEntry {
    /// Stable edit identifier; supply it when undoing or redoing to reject stale clients.
    pub id: i64,
    /// The workflow affected by this action, which may differ from the currently viewed one.
    pub workflow_id: i64,
    /// Human-readable action name, without an Undo or Redo prefix.
    pub label: String,
}

/// The next undo and redo actions shared by all workflows in a canonical project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct HistoryStatus {
    /// Canonical project folder owning this chronological history.
    pub project_path: PathBuf,
    /// Next undo action, or none at the baseline.
    pub undo: Option<HistoryEntry>,
    /// Next redo action, or none at the end of history.
    pub redo: Option<HistoryEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Edit {
    CreateNodeFromParents {
        node: Node,
        parents: Vec<NodeParent>,
    },
    RenameWorkflow {
        workflow_id: i64,
        before: String,
        after: String,
    },
    MoveNode {
        change: planning::Change,
    },
    DeleteNode {
        change: planning::Change,
    },
    CreateNode {
        node: Node,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent: Option<NodeParent>,
    },
    UpdatePrompt {
        before: Node,
        after: String,
    },
    CreateConnection {
        connection: Connection,
    },
}

impl Edit {
    fn workflow_id(&self) -> i64 {
        match self {
            Self::RenameWorkflow { workflow_id, .. } => *workflow_id,
            Self::MoveNode { change } | Self::DeleteNode { change } => change.workflow_id,
            Self::CreateNode { node, .. } | Self::CreateNodeFromParents { node, .. } => {
                node.workflow_id
            }
            Self::UpdatePrompt { before, .. } => before.workflow_id,
            Self::CreateConnection { connection } => connection.workflow_id,
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::RenameWorkflow { .. } => "Rename workflow",
            Self::MoveNode { .. } => "Move prompt node",
            Self::DeleteNode { .. } => "Delete prompt node",
            Self::CreateNode { .. } | Self::CreateNodeFromParents { .. } => "Create node",
            Self::UpdatePrompt { .. } => "Edit prompt",
            Self::CreateConnection { .. } => "Connect nodes",
        }
    }
}

enum Direction {
    Undo,
    Redo,
}

impl Storage {
    pub(super) async fn initialize_history(&self) -> anyhow::Result<()> {
        let mut connection = self.database.connect()?;
        let tx = connection.transaction().await?;
        let result = async {
            tx.execute(
                "CREATE TABLE IF NOT EXISTS workflow_history_heads (
                    project_path TEXT PRIMARY KEY,
                    cursor INTEGER NOT NULL CHECK (cursor >= 0)
                )",
                (),
            )
            .await?;
            tx.execute(
                "CREATE TABLE IF NOT EXISTS workflow_edits (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    project_path TEXT NOT NULL,
                    position INTEGER NOT NULL CHECK (position > 0),
                    version INTEGER NOT NULL,
                    record TEXT NOT NULL,
                    UNIQUE (project_path, position)
                )",
                (),
            )
            .await?;
            // Keep identities reserved even while creation is undone. Another
            // project's edits must never take an ID still needed by redo.
            tx.execute(
                "CREATE TABLE IF NOT EXISTS node_identities (id INTEGER PRIMARY KEY AUTOINCREMENT)",
                (),
            )
            .await?;
            tx.execute(
                "INSERT OR IGNORE INTO node_identities (id) SELECT id FROM nodes",
                (),
            )
            .await?;
            Ok(())
        }
        .await;
        finish(tx, result).await
    }

    pub(super) async fn create_node_with_history(
        &self,
        workflow_id: i64,
        prompt: &str,
        parents: &[NodeParent],
    ) -> anyhow::Result<Node> {
        self.ensure_dependency_operations_clear().await?;
        let mut connection = self.database.connect()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .await?;
        let result = async {
            let project = project_for_workflow(&tx, workflow_id).await?;
            dependencies::validate_parents(&tx, workflow_id, parents).await?;
            tx.execute("INSERT INTO node_identities DEFAULT VALUES", ())
                .await?;
            let node = Node {
                id: tx.last_insert_rowid(),
                workflow_id,
                prompt: prompt.to_owned(),
            };
            insert_node(&tx, &node).await?;
            for parent in parents {
                dependencies::attach(&tx, workflow_id, node.id, parent).await?;
            }
            let edit = if parents.len() > 1 {
                Edit::CreateNodeFromParents {
                    node: node.clone(),
                    parents: parents.to_vec(),
                }
            } else {
                Edit::CreateNode {
                    node: node.clone(),
                    parent: parents.first().cloned(),
                }
            };
            append(&tx, &project, &edit).await?;
            Ok(node)
        }
        .await;
        finish(tx, result).await
    }

    pub(super) async fn update_node_with_history(
        &self,
        workflow_id: i64,
        node_id: i64,
        prompt: &str,
        expected_prompt: Option<&str>,
    ) -> anyhow::Result<Node> {
        self.ensure_dependency_operations_clear().await?;
        let mut connection = self.database.connect()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .await?;
        let result =
            async {
                let project = project_for_workflow(&tx, workflow_id).await?;
                let mut rows = tx.query(
                "SELECT id, workflow_id, prompt FROM nodes WHERE workflow_id = ?1 AND id = ?2
                 AND NOT EXISTS (SELECT 1 FROM deleted_nodes WHERE node_id = nodes.id)",
                [workflow_id, node_id],
            ).await?;
                let row = rows
                    .next()
                    .await?
                    .ok_or(super::StorageFailure::MissingNode {
                        workflow_id,
                        node_id,
                    })?;
                let before = decode_node(&row)?;
                drop(rows);
                if expected_prompt.is_some_and(|expected| expected != before.prompt) {
                    return Err(super::StorageFailure::PromptConflict { current: before }.into());
                }
                if before.prompt != prompt {
                    replace_prompt(&tx, &before, &before.prompt, prompt).await?;
                    append(
                        &tx,
                        &project,
                        &Edit::UpdatePrompt {
                            before,
                            after: prompt.to_owned(),
                        },
                    )
                    .await?;
                }
                Ok(Node {
                    id: node_id,
                    workflow_id,
                    prompt: prompt.to_owned(),
                })
            }
            .await;
        finish(tx, result).await
    }

    pub(super) async fn connect_nodes_with_history(
        &self,
        workflow_id: i64,
        source_id: i64,
        target_id: i64,
    ) -> anyhow::Result<Connection> {
        self.ensure_dependency_operations_clear().await?;
        let mut connection = self.database.connect()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .await?;
        let result = async {
            let project = project_for_workflow(&tx, workflow_id).await?;
            dependencies::require_legacy_connections(&tx, workflow_id).await?;
            let edge = Connection {
                workflow_id,
                source_id,
                target_id,
            };
            insert_connection(&tx, &edge).await?;
            append(&tx, &project, &Edit::CreateConnection { connection: edge }).await?;
            Ok(edge)
        }
        .await;
        finish(tx, result).await
    }

    /// Inspects pending metadata actions without changing history or querying jj.
    ///
    /// # Errors
    ///
    /// Returns an error for an inaccessible project, unsupported history format,
    /// or a database/decoding failure. Existing data starts at a non-undoable baseline.
    pub async fn history_status(&self, project: &Path) -> Result<HistoryStatus, StorageError> {
        Ok(async {
            let project = canonical_project(project)?;
            let mut connection = self.database.connect()?;
            let tx = connection.transaction().await?;
            let result = status(&tx, &project).await;
            finish(tx, result).await
        }
        .await?)
    }

    /// Undoes the next metadata edit only if its ID matches the caller's selection.
    ///
    /// Never changes files, native history, run records, or conversation memory.
    /// Undoing node creation is blocked if native/run references now depend on it.
    ///
    /// # Errors
    ///
    /// Returns an error for empty/stale history, unsupported records, dependent
    /// references, unexpected saved state, or storage failure; the cursor stays unchanged.
    pub async fn undo(
        &mut self,
        project: &Path,
        expected_edit_id: i64,
    ) -> Result<HistoryStatus, StorageError> {
        Ok(self
            .move_history(project, expected_edit_id, Direction::Undo)
            .await?)
    }

    /// Reapplies the next metadata edit with its recorded values and identities.
    ///
    /// # Errors
    ///
    /// Returns an error for empty/stale history, unsupported records, unexpected
    /// saved state, or storage failure. No agent or native side effect is repeated.
    pub async fn redo(
        &mut self,
        project: &Path,
        expected_edit_id: i64,
    ) -> Result<HistoryStatus, StorageError> {
        Ok(self
            .move_history(project, expected_edit_id, Direction::Redo)
            .await?)
    }

    async fn move_history(
        &self,
        project: &Path,
        expected_edit_id: i64,
        direction: Direction,
    ) -> anyhow::Result<HistoryStatus> {
        self.ensure_dependency_operations_clear().await?;
        let project = canonical_project(project)?;
        let mut connection = self.database.connect()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .await?;
        let result = async {
            let cursor = cursor(&tx, &project).await?;
            let position = match direction {
                Direction::Undo => cursor,
                Direction::Redo => cursor + 1,
            };
            let (entry, edit) = read_edit(&tx, &project, position)
                .await?
                .context("no metadata action available")?;
            anyhow::ensure!(
                entry.id == expected_edit_id,
                "history changed; refresh history status before undo/redo"
            );
            let owner = project_for_workflow(&tx, edit.workflow_id()).await?;
            anyhow::ensure!(
                owner == project,
                "history workflow belongs to a different project"
            );
            apply(&tx, &edit, &direction).await?;
            let next = match direction {
                Direction::Undo => cursor - 1,
                Direction::Redo => cursor + 1,
            };
            tx.execute(
                "UPDATE workflow_history_heads SET cursor = ?2 WHERE project_path = ?1",
                turso::params![project.to_str(), next],
            )
            .await?;
            status(&tx, &project).await
        }
        .await;
        finish(tx, result).await
    }
}

async fn finish<T>(tx: Transaction<'_>, result: anyhow::Result<T>) -> anyhow::Result<T> {
    match result {
        Ok(value) => {
            tx.commit()
                .await
                .context("committing metadata and history")?;
            Ok(value)
        }
        Err(error) => {
            tx.rollback()
                .await
                .context("rolling back metadata and history")?;
            Err(error)
        }
    }
}

async fn project_for_workflow(tx: &turso::Connection, id: i64) -> anyhow::Result<PathBuf> {
    let mut rows = tx
        .query("SELECT project_path FROM workflows WHERE id = ?1", [id])
        .await?;
    let row = rows
        .next()
        .await?
        .ok_or(super::StorageFailure::MissingWorkflow { workflow_id: id })?;
    Ok(PathBuf::from(row.get::<String>(0)?))
}

async fn cursor(tx: &turso::Connection, project: &Path) -> anyhow::Result<i64> {
    let mut rows = tx
        .query(
            "SELECT cursor FROM workflow_history_heads WHERE project_path = ?1",
            [project.to_str()],
        )
        .await?;
    Ok(match rows.next().await? {
        Some(row) => row.get(0)?,
        None => 0,
    })
}

async fn append(tx: &turso::Connection, project: &Path, edit: &Edit) -> anyhow::Result<()> {
    let cursor = cursor(tx, project).await?;
    tx.execute(
        "DELETE FROM workflow_edits WHERE project_path = ?1 AND position > ?2",
        turso::params![project.to_str(), cursor],
    )
    .await?;
    let version = if matches!(edit, Edit::CreateNodeFromParents { .. }) {
        5
    } else if matches!(edit, Edit::RenameWorkflow { .. }) {
        4
    } else if matches!(edit, Edit::MoveNode { .. } | Edit::DeleteNode { .. }) {
        3
    } else if matches!(
        edit,
        Edit::CreateNode {
            parent: Some(_),
            ..
        }
    ) {
        2
    } else {
        1
    };
    tx.execute(
        "INSERT INTO workflow_edits (project_path, position, version, record) VALUES (?1, ?2, ?3, ?4)",
        turso::params![project.to_str(), cursor + 1, version, serde_json::to_string(edit)?],
    ).await?;
    tx.execute(
        "INSERT INTO workflow_history_heads (project_path, cursor) VALUES (?1, ?2)
         ON CONFLICT(project_path) DO UPDATE SET cursor = excluded.cursor",
        turso::params![project.to_str(), cursor + 1],
    )
    .await?;
    Ok(())
}

async fn read_edit(
    tx: &turso::Connection,
    project: &Path,
    position: i64,
) -> anyhow::Result<Option<(HistoryEntry, Edit)>> {
    let mut rows = tx.query(
        "SELECT id, version, record FROM workflow_edits WHERE project_path = ?1 AND position = ?2",
        turso::params![project.to_str(), position],
    ).await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    let version: i64 = row.get(1)?;
    anyhow::ensure!(
        (1..=5).contains(&version),
        "unsupported workflow history version {version}; current workflows are preserved"
    );
    let edit: Edit =
        serde_json::from_str(&row.get::<String>(2)?).context("decoding workflow history")?;
    Ok(Some((
        HistoryEntry {
            id: row.get(0)?,
            workflow_id: edit.workflow_id(),
            label: edit.label().to_owned(),
        },
        edit,
    )))
}

async fn status(tx: &turso::Connection, project: &Path) -> anyhow::Result<HistoryStatus> {
    let cursor = cursor(tx, project).await?;
    Ok(HistoryStatus {
        project_path: project.to_owned(),
        undo: read_edit(tx, project, cursor)
            .await?
            .map(|(entry, _)| entry),
        redo: read_edit(tx, project, cursor + 1)
            .await?
            .map(|(entry, _)| entry),
    })
}

async fn apply(tx: &turso::Connection, edit: &Edit, direction: &Direction) -> anyhow::Result<()> {
    match (edit, direction) {
        (
            Edit::RenameWorkflow {
                workflow_id,
                before,
                after,
            },
            direction,
        ) => {
            let (expected, replacement) = match direction {
                Direction::Undo => (after, before),
                Direction::Redo => (before, after),
            };
            rename::replace_name(tx, *workflow_id, expected, replacement).await?;
        }
        (Edit::MoveNode { change } | Edit::DeleteNode { change }, direction) => {
            planning::apply(tx, change, direction).await?;
        }
        (Edit::CreateNodeFromParents { node, parents }, direction) => {
            apply_created_node(tx, node, parents, direction).await?;
        }
        (Edit::CreateNode { node, parent }, direction) => {
            apply_created_node(tx, node, parent.as_slice(), direction).await?;
        }
        (Edit::UpdatePrompt { before, after }, Direction::Undo) => {
            replace_prompt(tx, before, after, &before.prompt).await?;
        }
        (Edit::UpdatePrompt { before, after }, Direction::Redo) => {
            replace_prompt(tx, before, &before.prompt, after).await?;
        }
        (Edit::CreateConnection { connection }, Direction::Redo) => {
            dependencies::require_legacy_connections(tx, connection.workflow_id).await?;
            insert_connection(tx, connection).await?
        }
        (Edit::CreateConnection { connection }, Direction::Undo) => {
            dependencies::require_legacy_connections(tx, connection.workflow_id).await?;
            let changed = tx.execute(
                "DELETE FROM connections WHERE workflow_id = ?1 AND source_id = ?2 AND target_id = ?3",
                [connection.workflow_id, connection.source_id, connection.target_id],
            ).await?;
            anyhow::ensure!(
                changed == 1,
                "cannot undo connection: saved connection changed"
            );
        }
    }
    Ok(())
}

async fn apply_created_node(
    tx: &turso::Connection,
    node: &Node,
    parents: &[NodeParent],
    direction: &Direction,
) -> anyhow::Result<()> {
    match direction {
        Direction::Redo => {
            dependencies::validate_parents(tx, node.workflow_id, parents).await?;
            insert_node(tx, node).await?;
            for parent in parents {
                dependencies::attach(tx, node.workflow_id, node.id, parent).await?;
            }
        }
        Direction::Undo => {
            dependencies::detach_created_node(tx, node.workflow_id, node.id, parents).await?;
            // Never delete associations, allocations, run history, or creation
            // guards to make metadata undo succeed. Those are separate lifecycles.
            let changed = tx
                .execute(
                    "DELETE FROM nodes WHERE id = ?1 AND workflow_id = ?2 AND prompt = ?3
                 AND NOT EXISTS (SELECT 1 FROM node_changes WHERE node_id = ?1)
                 AND NOT EXISTS (SELECT 1 FROM node_change_creations WHERE node_id = ?1)
                 AND NOT EXISTS (SELECT 1 FROM node_workspaces WHERE node_id = ?1)
                 AND NOT EXISTS (SELECT 1 FROM agent_runs WHERE node_id = ?1)
                 AND NOT EXISTS (SELECT 1 FROM connections WHERE source_id = ?1 OR target_id = ?1)",
                    turso::params![node.id, node.workflow_id, node.prompt.clone()],
                )
                .await?;
            anyhow::ensure!(
                changed == 1,
                "cannot undo node creation: saved node changed or native/run/connection references depend on it"
            );
        }
    }
    Ok(())
}

async fn insert_node(tx: &turso::Connection, node: &Node) -> anyhow::Result<()> {
    tx.execute(
        "INSERT INTO nodes (id, workflow_id, prompt) VALUES (?1, ?2, ?3)",
        turso::params![node.id, node.workflow_id, node.prompt.clone()],
    )
    .await
    .context("creating node")?;
    Ok(())
}

async fn replace_prompt(
    tx: &turso::Connection,
    node: &Node,
    expected: &str,
    replacement: &str,
) -> anyhow::Result<()> {
    let changed = tx
        .execute(
            "UPDATE nodes SET prompt = ?4 WHERE id = ?1 AND workflow_id = ?2 AND prompt = ?3",
            turso::params![node.id, node.workflow_id, expected, replacement],
        )
        .await?;
    anyhow::ensure!(changed == 1, "cannot edit prompt: saved node changed");
    Ok(())
}

async fn insert_connection(tx: &turso::Connection, edge: &Connection) -> anyhow::Result<()> {
    // One validation rule for both creation and redo. UNION deduplicates
    // reachable nodes in a DAG with converging paths.
    let changed = tx
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
            [edge.workflow_id, edge.source_id, edge.target_id],
        )
        .await
        .context("connecting nodes")?;
    anyhow::ensure!(
        changed == 1,
        "invalid connection {} -> {} in workflow {}: missing workflow or endpoints, self-link, duplicate link, or cycle",
        edge.source_id,
        edge.target_id,
        edge.workflow_id
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
    fn rooted_history_failures_roll_back_nodes_and_all_dependency_rows() -> anyhow::Result<()> {
        run(async {
            let directory = tempfile::tempdir()?;
            let mut storage = Storage::open(&directory.path().join("workflow.db")).await?;
            let base = crate::workflow::WorkflowBase {
                repository_path: directory.path().to_owned(),
                source: crate::workflow::BaseSource::Revision {
                    id: "root()".to_owned(),
                },
                revision: crate::jj::Revision {
                    change_id: "k".repeat(32),
                    commit_id: "0".repeat(40),
                    description: String::new(),
                    conflicted: false,
                },
            };
            let workflow = storage
                .create_workflow_record(directory.path(), "Rooted", Some(base))
                .await?;
            let root = storage
                .create_node_from(workflow.id, "Root", &NodeParent::WorkflowBase {})
                .await?;
            let expected = storage.workflow_graph(workflow.id).await?;
            let history = storage.history_status(directory.path()).await?;
            let connection = storage.database.connect()?;
            connection.execute("CREATE UNIQUE INDEX fail_rooted_history ON workflow_edits(version) WHERE version = 2", ()).await?;
            for parent in [
                NodeParent::WorkflowBase {},
                NodeParent::Node { node_id: root.id },
            ] {
                assert!(
                    storage
                        .create_node_from(workflow.id, "Must roll back", &parent)
                        .await
                        .is_err()
                );
                assert_eq!(storage.workflow_graph(workflow.id).await?, expected);
                assert_eq!(storage.history_status(directory.path()).await?, history);
            }
            connection
                .execute("DROP INDEX fail_rooted_history", ())
                .await?;
            let edit = history.undo.expect("root creation");
            storage.undo(directory.path(), edit.id).await?;
            let empty = storage.workflow_graph(workflow.id).await?;
            assert!(
                empty.nodes.is_empty()
                    && empty.base_nodes.is_empty()
                    && empty.connections.is_empty()
            );
            storage.redo(directory.path(), edit.id).await?;
            assert_eq!(storage.workflow_graph(workflow.id).await?, expected);
            Ok(())
        })
    }

    #[test]
    fn history_write_failure_rolls_back_edit_redo_tail_and_identity_allocation()
    -> anyhow::Result<()> {
        run(async {
            let directory = tempfile::tempdir()?;
            let mut storage = Storage::open(&directory.path().join("workflow.db")).await?;
            let workflow = storage
                .create_workflow(directory.path(), "Atomic edits")
                .await?;
            let a = storage.create_node(workflow.id, "A").await?;
            let b = storage.create_node(workflow.id, "B").await?;
            let latest = storage
                .history_status(directory.path())
                .await?
                .undo
                .expect("B action");
            let pending = storage.undo(directory.path(), latest.id).await?;
            let before = storage.workflow_graph(workflow.id).await?;
            let connection = storage.database.connect()?;
            // A real SQL constraint failure after the graph mutation and redo
            // deletion, rather than a mock error before the transaction begins.
            connection.execute("CREATE UNIQUE INDEX fail_history_insert ON workflow_edits(version) WHERE id <> 2", ()).await?;
            assert!(
                storage
                    .create_node(workflow.id, "Cannot save history")
                    .await
                    .is_err()
            );
            assert!(
                storage
                    .update_node(workflow.id, a.id, "Cannot save history")
                    .await
                    .is_err()
            );
            assert_eq!(storage.workflow_graph(workflow.id).await?, before);
            assert_eq!(storage.history_status(directory.path()).await?, pending);
            connection
                .execute("DROP INDEX fail_history_insert", ())
                .await?;
            storage.redo(directory.path(), latest.id).await?;
            assert_eq!(storage.node(workflow.id, b.id).await?, b);
            assert_eq!(storage.create_node(workflow.id, "C").await?.id, b.id + 1);
            Ok(())
        })
    }

    #[test]
    fn cursor_write_failure_rolls_back_undo_and_redo_mutations() -> anyhow::Result<()> {
        run(async {
            let directory = tempfile::tempdir()?;
            let mut storage = Storage::open(&directory.path().join("workflow.db")).await?;
            let workflow = storage
                .create_workflow(directory.path(), "Atomic cursor")
                .await?;
            let node = storage.create_node(workflow.id, "A").await?;
            let pending = storage.history_status(directory.path()).await?;
            let edit = pending.undo.as_ref().expect("create action").id;
            let connection = storage.database.connect()?;
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
            assert!(storage.undo(directory.path(), edit).await.is_err());
            assert_eq!(storage.node(workflow.id, node.id).await?, node);
            assert_eq!(storage.history_status(directory.path()).await?, pending);

            connection
                .execute("DROP TABLE workflow_history_heads", ())
                .await?;
            connection
                .execute(
                    "CREATE TABLE workflow_history_heads (
                    project_path TEXT PRIMARY KEY, cursor INTEGER NOT NULL CHECK(cursor = 0))",
                    (),
                )
                .await?;
            connection
                .execute(
                    "INSERT INTO workflow_history_heads VALUES (?1, 0)",
                    [pending.project_path.to_str()],
                )
                .await?;
            connection
                .execute("DELETE FROM nodes WHERE id = ?1", [node.id])
                .await?;
            let pending = storage.history_status(directory.path()).await?;
            assert!(storage.redo(directory.path(), edit).await.is_err());
            assert!(storage.workflow_graph(workflow.id).await?.nodes.is_empty());
            assert_eq!(storage.history_status(directory.path()).await?, pending);
            Ok(())
        })
    }

    #[test]
    fn unsupported_history_and_changed_state_fail_without_moving_cursor() -> anyhow::Result<()> {
        run(async {
            let directory = tempfile::tempdir()?;
            let mut storage = Storage::open(&directory.path().join("workflow.db")).await?;
            let workflow = storage.create_workflow(directory.path(), "Format").await?;
            let node = storage.create_node(workflow.id, "A").await?;
            let pending = storage.history_status(directory.path()).await?;
            let edit = pending.undo.as_ref().expect("create action").id;
            let connection = storage.database.connect()?;
            connection
                .execute("UPDATE workflow_edits SET version = 99", ())
                .await?;
            assert!(
                storage
                    .history_status(directory.path())
                    .await
                    .expect_err("unknown format")
                    .to_string()
                    .contains("unsupported workflow history version")
            );
            assert!(storage.undo(directory.path(), edit).await.is_err());
            assert_eq!(storage.node(workflow.id, node.id).await?, node);
            connection
                .execute("UPDATE workflow_edits SET version = 1", ())
                .await?;
            assert_eq!(storage.history_status(directory.path()).await?, pending);
            connection
                .execute("UPDATE nodes SET prompt = 'external metadata edit'", ())
                .await?;
            assert!(storage.undo(directory.path(), edit).await.is_err());
            assert_eq!(storage.history_status(directory.path()).await?, pending);
            Ok(())
        })
    }
}
