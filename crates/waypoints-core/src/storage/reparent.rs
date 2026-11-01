//! Checked native reparenting with durable uncertainty and project undo boundaries.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use turso::transaction::TransactionBehavior;

use super::dependencies::{self, require_rooted};
use super::{Storage, StorageError, WorkflowGraph};
use crate::jj::{self, ChangeState, Revision};
use crate::workflow::NodeParent;

mod deletion;
mod recovery;
pub use recovery::{DependencyRecovery, DependencyRecoveryAssessment};

/// Explicit consent to start a new metadata-undo baseline before changing code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum UndoBoundaryAcknowledgement {
    /// Workflow Undo cannot return to edits preceding this operation.
    AcceptUndoBoundary,
}

/// Whether native mutation and graph reconciliation are known to have completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DependencyOperationState {
    /// Reserved or interrupted; never automatically retried or cleared on restart.
    Unresolved,
    /// Native ancestry was verified and the graph was committed.
    Completed,
    /// Explicit recovery verified that native mutation never occurred.
    NotApplied,
}

/// The native mutation recorded by a dependency operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DependencyOperationKind {
    /// Move a change and its descendants onto a replacement dependency.
    #[default]
    Reparent,
    /// Abandon one change and reconnect its children onto its existing parent.
    Delete,
}

/// Durable exact inputs and observed outcome of a code-changing dependency edit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DependencyOperation {
    /// Database-local operation identity.
    pub id: i64,
    /// Native action; old records without this field are reparent operations.
    #[serde(default)]
    pub kind: DependencyOperationKind,
    /// Workflow whose graph is being changed.
    pub workflow_id: i64,
    /// Root node of the native edit; removed from the active graph for deletion.
    pub node_id: i64,
    /// Replacement dependency for reparenting, or inherited parent for deletion.
    pub parent: NodeParent,
    /// Saved dependency before the operation.
    pub previous_parent: NodeParent,
    /// Canonical native repository.
    pub repository_path: PathBuf,
    /// Exact operation against which preflight was performed.
    pub native_operation: String,
    /// Exact resulting native operation, present only after verified completion.
    pub resulting_native_operation: Option<String>,
    /// Exact source revision before mutation.
    pub source: Revision,
    /// Exact selected destination revision.
    pub destination: Revision,
    /// Native subtree before mutation; identities remain stable across rebasing.
    pub before: Vec<Revision>,
    /// Verified subtree after completion, including conflict flags.
    pub after: Vec<Revision>,
    /// Saved workspace to deregister for deletion; its directory and allocation history remain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deletion_workspace: Option<DeletionWorkspace>,
    /// Completion or durable uncertainty.
    pub state: DependencyOperationState,
    /// Diagnostic for inspection; not a retry instruction.
    pub detail: String,
}

/// Durable workspace deregistration intent and its verified native checkpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DeletionWorkspace {
    /// Exact saved workspace; never inferred from the current working directory.
    pub workspace: crate::workspace::WorkspaceAssociation,
    /// Operation after abandoning the source, before forgetting the workspace.
    pub abandoned_operation: Option<String>,
}

impl Storage {
    pub(super) async fn initialize_dependency_operations(&self) -> anyhow::Result<()> {
        let connection = self.database.connect()?;
        connection
            .execute(
                "CREATE TABLE IF NOT EXISTS dependency_operations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                workflow_id INTEGER NOT NULL REFERENCES workflows(id),
                active INTEGER NOT NULL CHECK (active IN (0, 1)),
                record TEXT NOT NULL
            )",
                (),
            )
            .await?;
        // One unresolved operation blocks all mutations in this database. This
        // conservative reservation also covers aliases and other saved workflows.
        connection
            .execute(
                "CREATE UNIQUE INDEX IF NOT EXISTS one_dependency_operation
             ON dependency_operations(active) WHERE active = 1",
                (),
            )
            .await?;
        connection
            .execute(
                "CREATE TABLE IF NOT EXISTS deleted_nodes (
                node_id INTEGER PRIMARY KEY REFERENCES nodes(id),
                operation_id INTEGER NOT NULL REFERENCES dependency_operations(id)
            )",
                (),
            )
            .await?;
        Ok(())
    }

    pub(super) async fn ensure_dependency_operations_clear(&self) -> anyhow::Result<()> {
        self.ensure_restores_clear().await?;
        let connection = self.database.connect()?;
        let mut rows = connection
            .query(
                "SELECT id, workflow_id FROM dependency_operations WHERE active = 1",
                (),
            )
            .await?;
        if let Some(row) = rows.next().await? {
            let id: i64 = row.get(0)?;
            let workflow_id: i64 = row.get(1)?;
            anyhow::bail!(
                "dependency operation {id} is unresolved; inspect node operations --workflow {workflow_id} and native jj state; do not retry or start conflicting work"
            );
        }
        Ok(())
    }

    /// Lists durable dependency operations without changing files or native state.
    ///
    /// # Errors
    ///
    /// Returns an error for a missing workflow or invalid stored records.
    pub async fn dependency_operations(
        &self,
        workflow_id: i64,
    ) -> Result<Vec<DependencyOperation>, StorageError> {
        self.workflow(workflow_id).await?;
        Ok(async {
            let connection = self.database.connect()?;
            let mut rows = connection
                .query(
                    "SELECT record FROM dependency_operations WHERE workflow_id = ?1 ORDER BY id",
                    [workflow_id],
                )
                .await?;
            let mut operations = Vec::new();
            while let Some(row) = rows.next().await? {
                operations.push(serde_json::from_str(&row.get::<String>(0)?)?);
            }
            Ok::<_, anyhow::Error>(operations)
        }
        .await?)
    }

    /// Rebases a native node and descendants onto one explicitly selected dependency.
    ///
    /// Requires acknowledgement even from non-CLI callers. Preflight refuses cycles,
    /// active/unresolved work, ancestry mismatches, and native descendants outside this
    /// workflow. No checkout is snapshotted or updated. Conflicts are successful native
    /// outcomes. Uncertain outcomes retain a durable reservation across restarts.
    ///
    /// # Errors
    ///
    /// Returns an error for missing acknowledgement, invalid/unsafe state, native
    /// failures, or storage failures. Once reserved, prior project undo/redo is gone;
    /// errors never imply rollback of native code and must not be blindly retried.
    pub async fn reparent_node(
        &mut self,
        workflow_id: i64,
        node_id: i64,
        parent: &NodeParent,
        acknowledgement: Option<UndoBoundaryAcknowledgement>,
    ) -> Result<DependencyOperation, StorageError> {
        if acknowledgement != Some(UndoBoundaryAcknowledgement::AcceptUndoBoundary) {
            return Err(anyhow::anyhow!(
                "reparenting changes code; pass --accept-undo-boundary: workflow Undo cannot cross this operation"
            ).into());
        }
        self.ensure_dependency_operations_clear().await?;
        let operation = tokio::time::timeout(
            Duration::from_secs(40),
            self.preflight_reparent(workflow_id, node_id, parent),
        )
        .await
        .context("reparent preflight timed out; no mutation started")??;
        self.execute_dependency_operation(operation).await
    }

    async fn execute_dependency_operation(
        &self,
        mut operation: DependencyOperation,
    ) -> Result<DependencyOperation, StorageError> {
        self.reserve_reparent(&mut operation).await?;
        let result = tokio::time::timeout(Duration::from_secs(70), async {
            anyhow::ensure!(
                jj::operation_id(&operation.repository_path).await? == operation.native_operation,
                "native repository changed after preflight"
            );
            match operation.kind {
                DependencyOperationKind::Reparent => {
                    jj::rebase_subtree(
                        &operation.repository_path,
                        &operation.source,
                        &operation.destination,
                        &operation.native_operation,
                    )
                    .await?
                }
                DependencyOperationKind::Delete => {
                    jj::abandon_change(
                        &operation.repository_path,
                        &operation.source,
                        &operation.native_operation,
                    )
                    .await?;
                    if operation.deletion_workspace.is_some() {
                        self.checkpoint_abandoned_deletion(&mut operation).await?;
                        self.forget_deletion_workspace(&operation).await?;
                    }
                }
            }
            self.finish_reparent(&mut operation).await
        })
        .await
        .context("native dependency edit timed out; outcome is uncertain")
        .and_then(|result| result);
        match result {
            Ok(()) => Ok(operation),
            Err(error) => {
                let detail = format!("{error:#}");
                operation.detail = detail.clone();
                // If reporting also fails, the original unresolved reservation
                // still exists with exact inputs and its initial warning.
                if let (Ok(connection), Ok(record)) =
                    (self.database.connect(), serde_json::to_string(&operation))
                {
                    let _ = connection.execute(
                        "UPDATE dependency_operations SET record = ?2 WHERE id = ?1 AND active = 1",
                        turso::params![operation.id, record],
                    ).await;
                }
                Err(anyhow::anyhow!(
                    "dependency operation {} remains unresolved: {detail}; inspect node operations and jj; do not retry",
                    operation.id
                ).into())
            }
        }
    }

    async fn preflight_reparent(
        &self,
        workflow_id: i64,
        node_id: i64,
        parent: &NodeParent,
    ) -> anyhow::Result<DependencyOperation> {
        self.preflight_dependency_edit(
            workflow_id,
            node_id,
            parent,
            DependencyOperationKind::Reparent,
        )
        .await
    }

    async fn preflight_dependency_edit(
        &self,
        workflow_id: i64,
        node_id: i64,
        parent: &NodeParent,
        kind: DependencyOperationKind,
    ) -> anyhow::Result<DependencyOperation> {
        let graph = self.workflow_graph(workflow_id).await?;
        require_rooted(&graph)?;
        anyhow::ensure!(
            graph.nodes.iter().any(|n| n.id == node_id),
            "node not found"
        );
        let base = graph
            .workflow
            .base
            .as_ref()
            .context("workflow has no base")?;
        let root = jj::repository_root(&graph.workflow.project_path).await?;
        anyhow::ensure!(
            root == base.repository_path,
            "native repository root changed"
        );
        let native_operation = jj::operation_id(&root).await?;
        let affected = graph_descendants(&graph, node_id);
        if let NodeParent::Node { node_id: upstream } = parent {
            anyhow::ensure!(
                !affected.contains(upstream),
                "replacement dependency would create a cycle"
            );
            self.node(workflow_id, *upstream).await?;
        }
        let old_parents: Vec<_> = graph
            .connections
            .iter()
            .filter(|edge| edge.target_id == node_id)
            .map(|edge| edge.source_id)
            .collect();
        anyhow::ensure!(
            old_parents.len() + usize::from(graph.base_nodes.contains(&node_id)) == 1,
            "native dependency editing currently requires exactly one existing dependency"
        );
        if kind == DependencyOperationKind::Reparent {
            anyhow::ensure!(
                match parent {
                    NodeParent::WorkflowBase {} => !graph.base_nodes.contains(&node_id),
                    NodeParent::Node { node_id } => old_parents != [*node_id],
                },
                "node already has that dependency"
            );
        }
        // Check all reservations, including other workflows, before reserving the
        // database globally. A finalizing/interrupted writer is still a writer.
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
            "active runs or incomplete native allocations must be resolved first"
        );
        drop(rows);
        let mut current = BTreeMap::new();
        for node in &graph.nodes {
            let refreshed = self.refresh_node(workflow_id, node.id).await?;
            match refreshed.state {
                ChangeState::Unassociated => {}
                ChangeState::Current { revision } => {
                    self.validate_dependency_association(workflow_id, node.id, &root, &revision)
                        .await?;
                    if affected.contains(&node.id) {
                        jj::ensure_mutable(&root, &revision).await?;
                    }
                    current.insert(node.id, revision);
                }
                _ => anyhow::bail!("node {} has missing or divergent native work", node.id),
            }
        }
        let source = current
            .get(&node_id)
            .context("native dependency editing requires a native change; create it first")?
            .clone();
        let deletion_workspace = if kind == DependencyOperationKind::Delete {
            self.deletion_workspace_preflight(workflow_id, node_id, &root, &source)
                .await?
        } else {
            None
        };
        let destination = match parent {
            NodeParent::WorkflowBase {} => {
                jj::resolve_base(&root, &base.revision.commit_id).await?
            }
            NodeParent::Node { node_id } => current
                .get(node_id)
                .context("replacement parent needs a current native change")?
                .clone(),
        };
        let before = jj::descendants(&root, &source).await?;
        let expected: BTreeSet<_> = current
            .iter()
            .filter(|(id, _)| affected.contains(id))
            .map(|(_, rev)| rev.commit_id.clone())
            .collect();
        let actual: BTreeSet<_> = before.iter().map(|rev| rev.commit_id.clone()).collect();
        anyhow::ensure!(
            actual == expected,
            "native descendants differ from this workflow's subtree (including external/other-workflow work); reconcile explicitly before reparenting"
        );
        anyhow::ensure!(
            !actual.contains(&destination.commit_id),
            "native destination is in the source subtree"
        );
        anyhow::ensure!(
            jj::operation_id(&root).await? == native_operation,
            "repository changed during preflight"
        );
        Ok(DependencyOperation {
            id: 0,
            kind,
            workflow_id,
            node_id,
            parent: parent.clone(),
            previous_parent: old_parents
                .first()
                .map_or(NodeParent::WorkflowBase {}, |id| NodeParent::Node {
                    node_id: *id,
                }),
            repository_path: root,
            native_operation,
            resulting_native_operation: None,
            source,
            destination,
            before,
            after: Vec::new(),
            deletion_workspace,
            state: DependencyOperationState::Unresolved,
            detail: "Reserved before native mutation; outcome is not yet known. Do not retry."
                .to_owned(),
        })
    }

    async fn reserve_reparent(&self, operation: &mut DependencyOperation) -> anyhow::Result<()> {
        let graph = self.workflow_graph(operation.workflow_id).await?;
        let mut connection = self.database.connect()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .await?;
        let result = async {
            tx.execute(
                "INSERT INTO dependency_operations(workflow_id, active, record) VALUES (?1, 1, ?2)",
                turso::params![operation.workflow_id, serde_json::to_string(operation)?],
            )
            .await?;
            operation.id = tx.last_insert_rowid();
            tx.execute(
                "UPDATE dependency_operations SET record = ?2 WHERE id = ?1",
                turso::params![operation.id, serde_json::to_string(operation)?],
            )
            .await?;
            tx.execute(
                "DELETE FROM workflow_edits WHERE project_path = ?1",
                [graph.workflow.project_path.to_str()],
            )
            .await?;
            tx.execute(
                "INSERT INTO workflow_history_heads(project_path, cursor) VALUES (?1, 0)
                ON CONFLICT(project_path) DO UPDATE SET cursor = 0",
                [graph.workflow.project_path.to_str()],
            )
            .await?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        match result {
            Ok(()) => tx.commit().await?,
            Err(error) => {
                tx.rollback().await?;
                return Err(error);
            }
        }
        Ok(())
    }

    async fn finish_reparent(&self, operation: &mut DependencyOperation) -> anyhow::Result<()> {
        let (record, saved) = self.saved_dependency_operation(operation.id).await?;
        anyhow::ensure!(saved == *operation, "operation reservation changed");
        let (graph, completed) = self.verify_dependency_operation(operation).await?;
        self.commit_dependency_resolution(&record, &graph, &completed)
            .await?;
        *operation = completed;
        Ok(())
    }

    async fn verify_dependency_operation(
        &self,
        operation: &DependencyOperation,
    ) -> anyhow::Result<(WorkflowGraph, DependencyOperation)> {
        match operation.kind {
            DependencyOperationKind::Reparent => self.verify_reparent(operation).await,
            DependencyOperationKind::Delete => self.verify_deletion(operation).await,
        }
    }

    async fn pending_dependency_graph(
        &self,
        operation: &DependencyOperation,
    ) -> anyhow::Result<WorkflowGraph> {
        let graph = self.workflow_graph(operation.workflow_id).await?;
        require_rooted(&graph)?;
        anyhow::ensure!(
            graph.unresolved_dependency_operation == Some(operation.id),
            "operation does not own the graph reservation"
        );
        let mut parents = Vec::new();
        if graph.base_nodes.contains(&operation.node_id) {
            parents.push(NodeParent::WorkflowBase {});
        }
        parents.extend(
            graph
                .connections
                .iter()
                .filter(|edge| edge.target_id == operation.node_id)
                .map(|edge| NodeParent::Node {
                    node_id: edge.source_id,
                }),
        );
        anyhow::ensure!(
            parents == [operation.previous_parent.clone()],
            "saved dependency changed since reservation"
        );
        let root = jj::repository_root(&graph.workflow.project_path).await?;
        anyhow::ensure!(
            root == operation.repository_path
                && graph
                    .workflow
                    .base
                    .as_ref()
                    .is_some_and(|base| base.repository_path == root),
            "native repository no longer matches the operation"
        );
        Ok(graph)
    }

    async fn current_graph_revisions(
        &self,
        graph: &WorkflowGraph,
    ) -> anyhow::Result<BTreeMap<i64, Revision>> {
        let mut revisions = BTreeMap::new();
        for node in &graph.nodes {
            let refreshed = self.refresh_node(graph.workflow.id, node.id).await?;
            match refreshed.state {
                ChangeState::Unassociated => {}
                ChangeState::Current { revision } => {
                    revisions.insert(node.id, revision);
                }
                _ => anyhow::bail!("native change disappeared or diverged during reconciliation"),
            }
        }
        Ok(revisions)
    }

    async fn verify_reparent(
        &self,
        operation: &DependencyOperation,
    ) -> anyhow::Result<(WorkflowGraph, DependencyOperation)> {
        let root = &operation.repository_path;
        let after_operation = jj::operation_id(root).await?;
        anyhow::ensure!(
            jj::operation_parent_ids(root).await? == [operation.native_operation.clone()],
            "unexpected native operations occurred between preflight and reconciliation"
        );
        let graph = self.pending_dependency_graph(operation).await?;
        let revisions = self.current_graph_revisions(&graph).await?;
        verify_reserved_dependencies(operation, &graph, &revisions)?;
        let mut expected_graph = graph.clone();
        expected_graph
            .base_nodes
            .retain(|id| *id != operation.node_id);
        expected_graph
            .connections
            .retain(|edge| edge.target_id != operation.node_id);
        match &operation.parent {
            NodeParent::WorkflowBase {} => expected_graph.base_nodes.push(operation.node_id),
            NodeParent::Node { node_id } => expected_graph.connections.push(super::Connection {
                workflow_id: operation.workflow_id,
                source_id: *node_id,
                target_id: operation.node_id,
            }),
        }
        verify_graph_ancestry(root, &expected_graph, &revisions).await?;
        let source = revisions
            .get(&operation.node_id)
            .context("source missing")?;
        anyhow::ensure!(
            source.change_id == operation.source.change_id,
            "source association changed"
        );
        let after = jj::descendants(root, source).await?;
        let before_ids: BTreeSet<_> = operation.before.iter().map(|rev| &rev.change_id).collect();
        let after_ids: BTreeSet<_> = after.iter().map(|rev| &rev.change_id).collect();
        anyhow::ensure!(
            before_ids == after_ids && before_ids.len() == after.len(),
            "native subtree identities changed unexpectedly"
        );
        anyhow::ensure!(
            jj::operation_id(root).await? == after_operation,
            "repository changed during reconciliation"
        );
        let mut completed = operation.clone();
        completed.resulting_native_operation = Some(after_operation);
        completed.after = after;
        completed.state = DependencyOperationState::Completed;
        completed.detail = "Native rebase and graph reconciliation completed. Physical checkouts were not updated; verify them before running agents.".to_owned();
        Ok((graph, completed))
    }

    async fn commit_dependency_resolution(
        &self,
        expected_record: &str,
        graph: &WorkflowGraph,
        resolved: &DependencyOperation,
    ) -> anyhow::Result<()> {
        let mut connection = self.database.connect()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .await?;
        let result = async {
            anyhow::ensure!(dependencies::load_graph(&tx, resolved.workflow_id).await? == *graph, "saved graph changed during reconciliation");
            if resolved.state == DependencyOperationState::Completed {
                match resolved.kind {
                    DependencyOperationKind::Reparent => {
                        tx.execute("DELETE FROM workflow_base_nodes WHERE workflow_id = ?1 AND node_id = ?2",
                            [resolved.workflow_id, resolved.node_id]).await?;
                        tx.execute("DELETE FROM connections WHERE workflow_id = ?1 AND target_id = ?2",
                            [resolved.workflow_id, resolved.node_id]).await?;
                        dependencies::attach(&tx, resolved.workflow_id, resolved.node_id, &resolved.parent).await?;
                    }
                    DependencyOperationKind::Delete => {
                        self.commit_node_deletion(&tx, graph, resolved).await?;
                    }
                }
                require_rooted(&dependencies::load_graph(&tx, resolved.workflow_id).await?)?;
            }
            let changed = tx.execute(
                "UPDATE dependency_operations SET active = 0, record = ?2 WHERE id = ?1 AND active = 1 AND record = ?3",
                turso::params![resolved.id, serde_json::to_string(resolved)?, expected_record],
            ).await?;
            anyhow::ensure!(changed == 1, "operation reservation changed");
            Ok::<_, anyhow::Error>(())
        }.await;
        match result {
            Ok(()) => tx.commit().await?,
            Err(error) => {
                tx.rollback().await?;
                return Err(error);
            }
        }
        Ok(())
    }
}

fn verify_reserved_dependencies(
    operation: &DependencyOperation,
    graph: &WorkflowGraph,
    revisions: &BTreeMap<i64, Revision>,
) -> anyhow::Result<()> {
    let destination = match &operation.parent {
        NodeParent::WorkflowBase {} => {
            &graph
                .workflow
                .base
                .as_ref()
                .context("base missing")?
                .revision
        }
        NodeParent::Node { node_id } => revisions
            .get(node_id)
            .context("replacement parent association missing")?,
    };
    anyhow::ensure!(
        *destination == operation.destination,
        "replacement dependency changed since reservation"
    );
    let affected = graph_descendants(graph, operation.node_id);
    let mapped: Vec<_> = revisions
        .iter()
        .filter(|(id, _)| affected.contains(id))
        .map(|(_, revision)| &revision.change_id)
        .collect();
    let mapped_ids: BTreeSet<_> = mapped.iter().copied().collect();
    let before_ids: BTreeSet<_> = operation
        .before
        .iter()
        .map(|revision| &revision.change_id)
        .collect();
    anyhow::ensure!(
        mapped_ids == before_ids && mapped.len() == operation.before.len(),
        "saved subtree associations changed since reservation"
    );
    Ok(())
}

async fn verify_graph_ancestry(
    root: &std::path::Path,
    graph: &WorkflowGraph,
    revisions: &BTreeMap<i64, Revision>,
) -> anyhow::Result<()> {
    for (id, revision) in revisions {
        let mut expected = Vec::new();
        if graph.base_nodes.contains(id) {
            expected.push(
                graph
                    .workflow
                    .base
                    .as_ref()
                    .context("base missing")?
                    .revision
                    .commit_id
                    .clone(),
            );
        }
        for edge in graph
            .connections
            .iter()
            .filter(|edge| edge.target_id == *id)
        {
            expected.push(
                revisions
                    .get(&edge.source_id)
                    .context("native parent missing")?
                    .commit_id
                    .clone(),
            );
        }
        expected.sort();
        expected.dedup();
        let mut actual = jj::parent_commit_ids(root, &revision.commit_id).await?;
        actual.sort();
        anyhow::ensure!(
            actual == expected,
            "native parents for node {id} do not match the proposed graph"
        );
    }
    Ok(())
}

fn graph_descendants(graph: &WorkflowGraph, root: i64) -> BTreeSet<i64> {
    let mut result = BTreeSet::from([root]);
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        for edge in graph
            .connections
            .iter()
            .filter(|edge| edge.source_id == node)
        {
            if result.insert(edge.target_id) {
                pending.push(edge.target_id);
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::BaseSource;

    pub(super) fn run(
        test: impl std::future::Future<Output = anyhow::Result<()>>,
    ) -> anyhow::Result<()> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(test)
    }

    pub(super) async fn fixture() -> anyhow::Result<(tempfile::TempDir, Storage, i64, i64)> {
        let directory = tempfile::tempdir()?;
        let output = std::process::Command::new("jj")
            .current_dir(directory.path())
            .args(["git", "init", "."])
            .output()?;
        anyhow::ensure!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mut storage = Storage::open(&directory.path().join("workflow.db")).await?;
        let workflow = storage
            .create_workflow_from(
                directory.path(),
                "Atomic reparent",
                &BaseSource::Revision {
                    id: "root()".to_owned(),
                },
            )
            .await?;
        let a = storage
            .create_node_from(workflow.id, "A", &NodeParent::WorkflowBase {})
            .await?;
        storage
            .create_node_change_from_dependency(workflow.id, a.id)
            .await?;
        let b = storage
            .create_node_from(workflow.id, "B", &NodeParent::Node { node_id: a.id })
            .await?;
        storage
            .create_node_change_from_dependency(workflow.id, b.id)
            .await?;
        Ok((directory, storage, workflow.id, b.id))
    }

    #[test]
    fn active_or_incomplete_work_blocks_reparent_before_the_undo_boundary() -> anyhow::Result<()> {
        run(async {
            let (directory, mut storage, workflow, node) = fixture().await?;
            let history = storage.history_status(directory.path()).await?;
            let native = jj::operation_id(directory.path()).await?;
            let connection = storage.database.connect()?;
            connection.execute(
                "INSERT INTO agent_runs(node_id, directory, active, record) VALUES (?1, 'test', 1, '{}')",
                [node],
            ).await?;
            assert!(
                storage
                    .reparent_node(
                        workflow,
                        node,
                        &NodeParent::WorkflowBase {},
                        Some(UndoBoundaryAcknowledgement::AcceptUndoBoundary)
                    )
                    .await
                    .expect_err("active run")
                    .to_string()
                    .contains("active runs")
            );
            connection.execute("DELETE FROM agent_runs", ()).await?;
            connection
                .execute(
                    "INSERT INTO node_change_creations(node_id) VALUES (?1)",
                    [node],
                )
                .await?;
            assert!(
                storage
                    .reparent_node(
                        workflow,
                        node,
                        &NodeParent::WorkflowBase {},
                        Some(UndoBoundaryAcknowledgement::AcceptUndoBoundary)
                    )
                    .await
                    .is_err()
            );
            assert_eq!(storage.history_status(directory.path()).await?, history);
            assert_eq!(jj::operation_id(directory.path()).await?, native);
            assert!(storage.dependency_operations(workflow).await?.is_empty());
            Ok(())
        })
    }

    #[test]
    fn reservation_failure_preserves_history_and_does_not_start_native_mutation()
    -> anyhow::Result<()> {
        run(async {
            let (directory, mut storage, workflow, node) = fixture().await?;
            let before = storage.workflow_graph(workflow).await?;
            let history = storage.history_status(directory.path()).await?;
            let native = jj::operation_id(directory.path()).await?;
            let connection = storage.database.connect()?;
            connection
                .execute(
                    "CREATE UNIQUE INDEX fail_reservation ON dependency_operations(workflow_id)",
                    (),
                )
                .await?;
            let mut prior = storage
                .preflight_reparent(workflow, node, &NodeParent::WorkflowBase {})
                .await?;
            prior.state = DependencyOperationState::Completed;
            connection.execute("INSERT INTO dependency_operations(workflow_id, active, record) VALUES (?1, 0, ?2)",
                turso::params![workflow, serde_json::to_string(&prior)?]).await?;
            assert!(
                storage
                    .reparent_node(
                        workflow,
                        node,
                        &NodeParent::WorkflowBase {},
                        Some(UndoBoundaryAcknowledgement::AcceptUndoBoundary)
                    )
                    .await
                    .is_err()
            );
            assert_eq!(storage.workflow_graph(workflow).await?, before);
            assert_eq!(storage.history_status(directory.path()).await?, history);
            assert_eq!(jj::operation_id(directory.path()).await?, native);
            Ok(())
        })
    }

    #[test]
    fn crash_after_reservation_survives_restart_and_blocks_mutations() -> anyhow::Result<()> {
        run(async {
            let (directory, storage, workflow, node) = fixture().await?;
            let native = jj::operation_id(directory.path()).await?;
            let mut operation = storage
                .preflight_reparent(workflow, node, &NodeParent::WorkflowBase {})
                .await?;
            storage.reserve_reparent(&mut operation).await?;
            drop(storage); // The same durable state as process death before spawning jj.
            let mut storage = Storage::open(&directory.path().join("workflow.db")).await?;
            assert_eq!(
                storage.dependency_operations(workflow).await?,
                vec![operation.clone()]
            );
            assert_eq!(
                storage
                    .workflow_graph(workflow)
                    .await?
                    .unresolved_dependency_operation,
                Some(operation.id)
            );
            assert_eq!(jj::operation_id(directory.path()).await?, native);
            assert!(
                storage
                    .history_status(directory.path())
                    .await?
                    .undo
                    .is_none()
            );
            assert!(
                storage
                    .update_node(workflow, node, "blocked")
                    .await
                    .is_err()
            );
            let saved_workflow = storage.workflow(workflow).await?;
            assert!(storage.rename_workflow(workflow, "blocked").await.is_err());
            assert_eq!(storage.workflow(workflow).await?, saved_workflow);
            assert!(
                storage
                    .create_node_change(workflow, node, "root()")
                    .await
                    .is_err()
            );
            assert!(storage.ensure_node_idle(node).await.is_err());
            assert!(
                storage
                    .create_node_from(workflow, "blocked", &NodeParent::WorkflowBase {})
                    .await
                    .is_err()
            );
            assert!(
                storage
                    .reparent_node(
                        workflow,
                        node,
                        &NodeParent::WorkflowBase {},
                        Some(UndoBoundaryAcknowledgement::AcceptUndoBoundary)
                    )
                    .await
                    .is_err()
            );
            Ok(())
        })
    }

    #[test]
    fn external_write_after_rebase_prevents_graph_completion() -> anyhow::Result<()> {
        run(async {
            let (directory, storage, workflow, node) = fixture().await?;
            let mut operation = storage
                .preflight_reparent(workflow, node, &NodeParent::WorkflowBase {})
                .await?;
            storage.reserve_reparent(&mut operation).await?;
            jj::rebase_subtree(
                directory.path(),
                &operation.source,
                &operation.destination,
                &operation.native_operation,
            )
            .await?;
            // A second native mutation in the gap must not be mistaken for the
            // result of our pinned operation, even if the node ancestry still fits.
            jj::create_change(directory.path(), &operation.destination).await?;
            let before = storage.workflow_graph(workflow).await?;
            let error = storage
                .finish_reparent(&mut operation)
                .await
                .expect_err("external operation");
            assert!(
                error.to_string().contains("unexpected native operations"),
                "{error}"
            );
            assert_eq!(storage.workflow_graph(workflow).await?, before);
            assert_eq!(
                storage.dependency_operations(workflow).await?[0].state,
                DependencyOperationState::Unresolved
            );
            Ok(())
        })
    }

    #[test]
    fn graph_write_failure_after_native_rebase_is_unresolved_without_partial_metadata()
    -> anyhow::Result<()> {
        run(async {
            let (directory, mut storage, workflow, node) = fixture().await?;
            let mut before = storage.workflow_graph(workflow).await?;
            let native = jj::operation_id(directory.path()).await?;
            let connection = storage.database.connect()?;
            // The initial root occupies this slot. Reconciliation must roll back
            // deletion of B's incoming edge when inserting its new base link fails.
            connection
                .execute(
                    "CREATE UNIQUE INDEX fail_second_root ON workflow_base_nodes(workflow_id)",
                    (),
                )
                .await?;
            let error = storage
                .reparent_node(
                    workflow,
                    node,
                    &NodeParent::WorkflowBase {},
                    Some(UndoBoundaryAcknowledgement::AcceptUndoBoundary),
                )
                .await
                .expect_err("SQL failure after native mutation");
            assert!(error.to_string().contains("remains unresolved"), "{error}");
            assert_ne!(jj::operation_id(directory.path()).await?, native);
            let operation = storage.dependency_operations(workflow).await?.remove(0);
            assert_eq!(operation.state, DependencyOperationState::Unresolved);
            before.unresolved_dependency_operation = Some(operation.id);
            assert_eq!(storage.workflow_graph(workflow).await?, before);
            let current = storage.refresh_node(workflow, node).await?;
            let ChangeState::Current { revision } = current.state else {
                panic!("current native change");
            };
            assert_eq!(
                jj::parent_commit_ids(directory.path(), &revision.commit_id).await?,
                vec![operation.destination.commit_id]
            );
            drop(storage);
            let storage = Storage::open(&directory.path().join("workflow.db")).await?;
            assert_eq!(storage.workflow_graph(workflow).await?, before);
            assert!(storage.ensure_node_idle(node).await.is_err());
            Ok(())
        })
    }

    #[test]
    fn recovery_releases_unapplied_operation_only_with_confirmation_and_keeps_boundary()
    -> anyhow::Result<()> {
        run(async {
            let (directory, storage, workflow, node) = fixture().await?;
            let mut operation = storage
                .preflight_reparent(workflow, node, &NodeParent::WorkflowBase {})
                .await?;
            storage.reserve_reparent(&mut operation).await?;
            let mut expected = storage.workflow_graph(workflow).await?;
            drop(storage);
            let mut storage = Storage::open(&directory.path().join("workflow.db")).await?;
            let inspection = storage.inspect_dependency_recovery(operation.id).await?;
            assert_eq!(
                inspection.assessment,
                DependencyRecoveryAssessment::NotApplied
            );
            assert_eq!(inspection.operation, operation);
            assert_eq!(
                storage.workflow_graph(workflow).await?,
                expected,
                "inspection never releases"
            );
            assert!(
                storage
                    .recover_dependency_operation(operation.id, None)
                    .await
                    .is_err()
            );
            assert!(storage.ensure_node_idle(node).await.is_err());
            let result = storage
                .recover_dependency_operation(
                    operation.id,
                    Some(crate::run::RecoveryConfirmation::ProcessesStopped),
                )
                .await?;
            assert_eq!(result.state, DependencyOperationState::NotApplied);
            assert!(result.after.is_empty() && result.resulting_native_operation.is_none());
            assert_eq!(
                jj::operation_id(directory.path()).await?,
                operation.native_operation
            );
            expected.unresolved_dependency_operation = None;
            assert_eq!(storage.workflow_graph(workflow).await?, expected);
            let history = storage.history_status(directory.path()).await?;
            assert!(
                history.undo.is_none() && history.redo.is_none(),
                "no resurrected history"
            );
            storage.update_node(workflow, node, "unblocked").await?;
            let edit = storage
                .history_status(directory.path())
                .await?
                .undo
                .expect("new prompt edit");
            storage.undo(directory.path(), edit.id).await?;
            assert!(
                storage
                    .history_status(directory.path())
                    .await?
                    .undo
                    .is_none()
            );
            drop(storage);
            let storage = Storage::open(&directory.path().join("workflow.db")).await?;
            assert_eq!(storage.dependency_operations(workflow).await?, vec![result]);
            Ok(())
        })
    }

    #[test]
    fn recovery_finishes_applied_rebase_after_restart_without_running_jj_again()
    -> anyhow::Result<()> {
        run(async {
            let (directory, storage, workflow, node) = fixture().await?;
            let mut operation = storage
                .preflight_reparent(workflow, node, &NodeParent::WorkflowBase {})
                .await?;
            storage.reserve_reparent(&mut operation).await?;
            jj::rebase_subtree(
                directory.path(),
                &operation.source,
                &operation.destination,
                &operation.native_operation,
            )
            .await?;
            let native = jj::operation_id(directory.path()).await?;
            std::fs::write(directory.path().join("dirty.txt"), "unrecorded work\n")?;
            drop(storage); // Crash after jj committed, before metadata completion.
            let mut storage = Storage::open(&directory.path().join("workflow.db")).await?;
            let before = storage.workflow_graph(workflow).await?;
            assert_eq!(
                storage
                    .inspect_dependency_recovery(operation.id)
                    .await?
                    .assessment,
                DependencyRecoveryAssessment::Applied
            );
            assert_eq!(storage.workflow_graph(workflow).await?, before);
            let result = storage
                .recover_dependency_operation(
                    operation.id,
                    Some(crate::run::RecoveryConfirmation::ProcessesStopped),
                )
                .await?;
            assert_eq!(result.state, DependencyOperationState::Completed);
            assert_eq!(result.resulting_native_operation, Some(native.clone()));
            assert_eq!(result.after.len(), operation.before.len());
            let graph = storage.workflow_graph(workflow).await?;
            assert!(
                graph.unresolved_dependency_operation.is_none() && graph.base_nodes.contains(&node)
            );
            assert!(graph.connections.iter().all(|edge| edge.target_id != node));
            assert_eq!(
                jj::operation_id(directory.path()).await?,
                native,
                "recovery is read-only in jj"
            );
            assert_eq!(
                std::fs::read_to_string(directory.path().join("dirty.txt"))?,
                "unrecorded work\n"
            );
            storage.ensure_node_idle(node).await?;
            assert_eq!(
                storage
                    .inspect_dependency_recovery(operation.id)
                    .await?
                    .assessment,
                DependencyRecoveryAssessment::Resolved
            );
            Ok(())
        })
    }

    #[test]
    fn recovery_rechecks_inspection_and_refuses_ambiguous_native_operations() -> anyhow::Result<()>
    {
        run(async {
            for applied in [false, true] {
                let (directory, mut storage, workflow, node) = fixture().await?;
                let mut operation = storage
                    .preflight_reparent(workflow, node, &NodeParent::WorkflowBase {})
                    .await?;
                storage.reserve_reparent(&mut operation).await?;
                assert_eq!(
                    storage
                        .inspect_dependency_recovery(operation.id)
                        .await?
                        .assessment,
                    DependencyRecoveryAssessment::NotApplied
                );
                if applied {
                    jj::rebase_subtree(
                        directory.path(),
                        &operation.source,
                        &operation.destination,
                        &operation.native_operation,
                    )
                    .await?;
                }
                jj::create_change(directory.path(), &operation.destination).await?;
                let native = jj::operation_id(directory.path()).await?;
                let before = storage.workflow_graph(workflow).await?;
                assert!(matches!(
                    storage
                        .inspect_dependency_recovery(operation.id)
                        .await?
                        .assessment,
                    DependencyRecoveryAssessment::Blocked { .. }
                ));
                assert!(
                    storage
                        .recover_dependency_operation(
                            operation.id,
                            Some(crate::run::RecoveryConfirmation::ProcessesStopped)
                        )
                        .await
                        .is_err()
                );
                assert_eq!(jj::operation_id(directory.path()).await?, native);
                assert_eq!(storage.workflow_graph(workflow).await?, before);
                assert_eq!(
                    storage.dependency_operations(workflow).await?,
                    vec![operation]
                );
                assert!(storage.ensure_node_idle(node).await.is_err());
            }
            Ok(())
        })
    }

    #[test]
    fn recovery_storage_failure_rolls_back_graph_and_keeps_reservation_for_retry()
    -> anyhow::Result<()> {
        run(async {
            for applied in [false, true] {
                let (directory, mut storage, workflow, node) = fixture().await?;
                let mut operation = storage
                    .preflight_reparent(workflow, node, &NodeParent::WorkflowBase {})
                    .await?;
                storage.reserve_reparent(&mut operation).await?;
                if applied {
                    jj::rebase_subtree(
                        directory.path(),
                        &operation.source,
                        &operation.destination,
                        &operation.native_operation,
                    )
                    .await?;
                }
                let native = jj::operation_id(directory.path()).await?;
                let before = storage.workflow_graph(workflow).await?;
                let connection = storage.database.connect()?;
                // Let edge reconciliation run, then fail reservation release.
                connection.execute(
                    "INSERT INTO dependency_operations(workflow_id, active, record) VALUES (?1, 0, '{}')",
                    [workflow],
                ).await?;
                connection.execute("CREATE UNIQUE INDEX fail_recovery_release ON dependency_operations(active)", ()).await?;
                assert!(
                    storage
                        .recover_dependency_operation(
                            operation.id,
                            Some(crate::run::RecoveryConfirmation::ProcessesStopped)
                        )
                        .await
                        .is_err()
                );
                assert_eq!(storage.workflow_graph(workflow).await?, before);
                assert_eq!(
                    storage.saved_dependency_operation(operation.id).await?.1,
                    operation
                );
                assert!(storage.ensure_node_idle(node).await.is_err());
                connection
                    .execute("DROP INDEX fail_recovery_release", ())
                    .await?;
                let result = storage
                    .recover_dependency_operation(
                        operation.id,
                        Some(crate::run::RecoveryConfirmation::ProcessesStopped),
                    )
                    .await?;
                assert_eq!(
                    result.state,
                    if applied {
                        DependencyOperationState::Completed
                    } else {
                        DependencyOperationState::NotApplied
                    }
                );
                assert_eq!(jj::operation_id(directory.path()).await?, native);
            }
            Ok(())
        })
    }

    #[test]
    fn repeated_recovery_of_old_operation_does_not_release_a_new_reservation() -> anyhow::Result<()>
    {
        run(async {
            let (_directory, mut storage, workflow, node) = fixture().await?;
            let mut old = storage
                .preflight_reparent(workflow, node, &NodeParent::WorkflowBase {})
                .await?;
            storage.reserve_reparent(&mut old).await?;
            let result = storage
                .recover_dependency_operation(
                    old.id,
                    Some(crate::run::RecoveryConfirmation::ProcessesStopped),
                )
                .await?;
            let mut new = storage
                .preflight_reparent(workflow, node, &NodeParent::WorkflowBase {})
                .await?;
            storage.reserve_reparent(&mut new).await?;
            assert_eq!(
                storage
                    .recover_dependency_operation(
                        old.id,
                        Some(crate::run::RecoveryConfirmation::ProcessesStopped)
                    )
                    .await?,
                result
            );
            assert_eq!(storage.saved_dependency_operation(new.id).await?.1, new);
            assert_eq!(
                storage
                    .workflow_graph(workflow)
                    .await?
                    .unresolved_dependency_operation,
                Some(new.id)
            );
            assert!(storage.ensure_node_idle(node).await.is_err());
            Ok(())
        })
    }

    #[test]
    fn changed_operation_record_cannot_release_reservation_or_partially_update_graph()
    -> anyhow::Result<()> {
        run(async {
            let (directory, storage, workflow, node) = fixture().await?;
            let mut operation = storage
                .preflight_reparent(workflow, node, &NodeParent::WorkflowBase {})
                .await?;
            storage.reserve_reparent(&mut operation).await?;
            jj::rebase_subtree(
                directory.path(),
                &operation.source,
                &operation.destination,
                &operation.native_operation,
            )
            .await?;
            let (record, _) = storage.saved_dependency_operation(operation.id).await?;
            let (graph, resolved) = storage.verify_reparent(&operation).await?;
            operation.detail = "A different saved intent".to_owned();
            storage
                .database
                .connect()?
                .execute(
                    "UPDATE dependency_operations SET record = ?2 WHERE id = ?1",
                    turso::params![operation.id, serde_json::to_string(&operation)?],
                )
                .await?;
            assert!(
                storage
                    .commit_dependency_resolution(&record, &graph, &resolved)
                    .await
                    .is_err()
            );
            assert_eq!(storage.workflow_graph(workflow).await?, graph);
            assert_eq!(
                storage.saved_dependency_operation(operation.id).await?.1,
                operation
            );
            Ok(())
        })
    }
}
