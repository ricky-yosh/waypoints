//! Native node deletion without workspace cleanup, using the shared operation lifecycle.

use std::collections::BTreeSet;
use std::time::Duration;

use anyhow::Context;

use super::{
    DependencyOperation, DependencyOperationKind, DependencyOperationState,
    UndoBoundaryAcknowledgement, verify_graph_ancestry, verify_reserved_dependencies,
};
use crate::jj::{self, ChangeState};
use crate::storage::{Connection, Storage, StorageError, WorkflowGraph};
use crate::workflow::NodeParent;

mod workspace;

impl Storage {
    /// Deregisters a node's saved workspace, abandons native work, and reconnects children.
    ///
    /// Retains the node's historical data, run records, and identity, but removes it
    /// from active graph/node APIs only after verifying native abandonment. Requires
    /// explicit undo-boundary consent. No files or physical checkouts are changed.
    ///
    /// # Errors
    ///
    /// Returns an error for missing consent, unrooted/unsafe work, mismatched or unsaved
    /// workspaces, multiple parents, or native/storage failure. Uncertain outcomes keep
    /// the shared reservation and require explicit operation recovery.
    pub async fn delete_node(
        &mut self,
        workflow_id: i64,
        node_id: i64,
        acknowledgement: Option<UndoBoundaryAcknowledgement>,
    ) -> Result<DependencyOperation, StorageError> {
        if acknowledgement != Some(UndoBoundaryAcknowledgement::AcceptUndoBoundary) {
            return Err(anyhow::anyhow!(
                "deletion abandons native code; pass --accept-undo-boundary: workflow Undo cannot cross this operation"
            ).into());
        }
        self.ensure_dependency_operations_clear().await?;
        let operation = tokio::time::timeout(
            Duration::from_secs(40),
            self.preflight_deletion(workflow_id, node_id),
        )
        .await
        .context("deletion preflight timed out; no mutation started")??;
        self.execute_dependency_operation(operation).await
    }

    pub(super) async fn preflight_deletion(
        &self,
        workflow_id: i64,
        node_id: i64,
    ) -> anyhow::Result<DependencyOperation> {
        let graph = self.workflow_graph(workflow_id).await?;
        let parent = if graph.base_nodes.contains(&node_id) {
            NodeParent::WorkflowBase {}
        } else {
            NodeParent::Node {
                node_id: graph
                    .connections
                    .iter()
                    .find(|edge| edge.target_id == node_id)
                    .context("node has no saved dependency")?
                    .source_id,
            }
        };
        self.preflight_dependency_edit(
            workflow_id,
            node_id,
            &parent,
            DependencyOperationKind::Delete,
        )
        .await
    }

    pub(super) async fn verify_deletion(
        &self,
        operation: &DependencyOperation,
    ) -> anyhow::Result<(WorkflowGraph, DependencyOperation)> {
        let root = &operation.repository_path;
        let mutation_parent = operation
            .deletion_workspace
            .as_ref()
            .and_then(|workspace| workspace.abandoned_operation.as_ref())
            .unwrap_or(&operation.native_operation);
        if operation.deletion_workspace.is_some() {
            anyhow::ensure!(
                mutation_parent != &operation.native_operation,
                "native abandonment has not been checkpointed; inspect operation recovery"
            );
            anyhow::ensure!(
                jj::operation_parent_ids_at(root, mutation_parent).await?
                    == [operation.native_operation.clone()],
                "native abandonment operation no longer matches preflight"
            );
        }
        self.verify_forgotten_registry(operation).await?;
        self.verify_deletion_result(operation, mutation_parent)
            .await
    }

    pub(super) async fn verify_deletion_result(
        &self,
        operation: &DependencyOperation,
        mutation_parent: &str,
    ) -> anyhow::Result<(WorkflowGraph, DependencyOperation)> {
        let root = &operation.repository_path;
        let native = jj::operation_id(root).await?;
        anyhow::ensure!(
            jj::operation_parent_ids(root).await? == [mutation_parent.to_owned()],
            "unexpected native operations occurred between preflight and reconciliation"
        );
        let graph = self.pending_dependency_graph(operation).await?;
        anyhow::ensure!(
            operation.parent == operation.previous_parent,
            "deletion's inherited dependency changed"
        );
        let source = self
            .refresh_node(operation.workflow_id, operation.node_id)
            .await?;
        anyhow::ensure!(
            matches!(source.state, ChangeState::Missing)
                && source
                    .association
                    .as_ref()
                    .is_some_and(
                        |association| association.change_id == operation.source.change_id
                            && association.repository_path == operation.repository_path
                    ),
            "deleted change is still visible or its saved association changed"
        );
        let expected_graph = deletion_graph(&graph, operation)?;
        let mut revisions = self.current_graph_revisions(&expected_graph).await?;
        // The abandoned change remains a historical association, not an active
        // native revision. Verify the reserved mapping with its exact old identity.
        revisions.insert(operation.node_id, operation.source.clone());
        verify_reserved_dependencies(operation, &graph, &revisions)?;
        revisions.remove(&operation.node_id);
        verify_graph_ancestry(root, &expected_graph, &revisions).await?;
        let before_ids: BTreeSet<_> = operation
            .before
            .iter()
            .filter(|revision| revision.change_id != operation.source.change_id)
            .map(|revision| &revision.change_id)
            .collect();
        let mut after: Vec<_> = revisions
            .values()
            .filter(|revision| before_ids.contains(&revision.change_id))
            .cloned()
            .collect();
        anyhow::ensure!(
            after.len() == before_ids.len(),
            "native descendant identities changed"
        );
        let mut descendants = BTreeSet::new();
        for child in graph
            .connections
            .iter()
            .filter(|edge| edge.source_id == operation.node_id)
        {
            if let Some(revision) = revisions.get(&child.target_id) {
                descendants.extend(
                    jj::descendants(root, revision)
                        .await?
                        .into_iter()
                        .map(|revision| revision.change_id),
                );
            }
        }
        anyhow::ensure!(
            descendants.iter().collect::<BTreeSet<_>>() == before_ids,
            "native descendants no longer match the saved deletion subtree"
        );
        anyhow::ensure!(
            jj::operation_id(root).await? == native,
            "repository changed during deletion reconciliation"
        );
        after.sort_by(|a, b| a.commit_id.cmp(&b.commit_id));
        let mut completed = operation.clone();
        completed.after = after;
        completed.resulting_native_operation = Some(native);
        completed.state = DependencyOperationState::Completed;
        completed.detail = "Native change abandoned and node removed from the active graph. Children retain their changes and inherit its parent. Files and physical checkouts were not updated.".to_owned();
        if let Some(workspace) = &operation.deletion_workspace {
            completed.detail.push_str(&format!(" Workspace {:?} deregistered; folder and all files left at {}. Its local .jj metadata is retained, not a usable registered workspace.",
                workspace.workspace.name, workspace.workspace.directory.display()));
        }
        Ok((graph, completed))
    }

    pub(super) async fn commit_node_deletion(
        &self,
        tx: &turso::Connection,
        graph: &WorkflowGraph,
        operation: &DependencyOperation,
    ) -> anyhow::Result<()> {
        let expected = deletion_graph(graph, operation)?;
        workspace::verify_allocation(tx, operation).await?;
        tx.execute(
            "DELETE FROM workflow_base_nodes WHERE workflow_id = ?1 AND node_id = ?2",
            [operation.workflow_id, operation.node_id],
        )
        .await?;
        tx.execute(
            "DELETE FROM connections WHERE workflow_id = ?1 AND (source_id = ?2 OR target_id = ?2)",
            [operation.workflow_id, operation.node_id],
        )
        .await?;
        for child in graph
            .connections
            .iter()
            .filter(|edge| edge.source_id == operation.node_id)
        {
            let already_attached = match operation.parent {
                NodeParent::WorkflowBase {} => graph.base_nodes.contains(&child.target_id),
                NodeParent::Node { node_id } => graph
                    .connections
                    .iter()
                    .any(|edge| edge.source_id == node_id && edge.target_id == child.target_id),
            };
            if !already_attached {
                super::dependencies::attach(
                    tx,
                    operation.workflow_id,
                    child.target_id,
                    &operation.parent,
                )
                .await?;
            }
        }
        // Keep foreign-key references, captured prompts/runs, and native IDs as
        // history. This marker is not a metadata-only archive: jj was abandoned.
        tx.execute(
            "INSERT INTO deleted_nodes(node_id, operation_id) VALUES (?1, ?2)",
            [operation.node_id, operation.id],
        )
        .await?;
        anyhow::ensure!(
            super::dependencies::load_graph(tx, operation.workflow_id).await? == expected,
            "saved deletion graph does not match the verified native result"
        );
        Ok(())
    }
}

fn deletion_graph(
    graph: &WorkflowGraph,
    operation: &DependencyOperation,
) -> anyhow::Result<WorkflowGraph> {
    anyhow::ensure!(
        graph.nodes.iter().any(|node| node.id == operation.node_id),
        "node not found in active graph"
    );
    let mut result = graph.clone();
    result.nodes.retain(|node| node.id != operation.node_id);
    result.base_nodes.retain(|id| *id != operation.node_id);
    result
        .connections
        .retain(|edge| edge.source_id != operation.node_id && edge.target_id != operation.node_id);
    for child in graph
        .connections
        .iter()
        .filter(|edge| edge.source_id == operation.node_id)
    {
        match operation.parent {
            NodeParent::WorkflowBase {} => {
                if !result.base_nodes.contains(&child.target_id) {
                    result.base_nodes.push(child.target_id);
                }
            }
            NodeParent::Node { node_id } => {
                if !result
                    .connections
                    .iter()
                    .any(|edge| edge.source_id == node_id && edge.target_id == child.target_id)
                {
                    result.connections.push(Connection {
                        workflow_id: operation.workflow_id,
                        source_id: node_id,
                        target_id: child.target_id,
                    });
                }
            }
        }
    }
    result.base_nodes.sort();
    result
        .connections
        .sort_by_key(|edge| (edge.source_id, edge.target_id));
    Ok(result)
}

#[cfg(test)]
mod tests;
