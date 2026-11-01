//! Metadata-only topology edits with local before/after state and native-reference guards.

use std::collections::BTreeSet;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use turso::transaction::TransactionBehavior;

use super::{Direction, Edit, append, dependencies, finish, insert_connection, insert_node};
use crate::storage::{Connection, Node, Storage, StorageError, WorkflowGraph};
use crate::workflow::NodeParent;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Attachment {
    node_id: i64,
    parents: Vec<NodeParent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct State {
    node: Option<Node>,
    attachments: Vec<Attachment>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Change {
    pub(super) workflow_id: i64,
    node_id: i64,
    before: State,
    after: State,
}

impl Storage {
    /// Moves a prompt-only subtree to one parent without invoking jj.
    ///
    /// # Errors
    ///
    /// Refuses unrooted graphs, cycles, invalid parents, multiple source parents,
    /// native/run/workspace references anywhere in the subtree, or storage failures.
    pub async fn move_prompt_node(
        &mut self,
        workflow_id: i64,
        node_id: i64,
        parent: &NodeParent,
    ) -> Result<WorkflowGraph, StorageError> {
        Ok(self
            .edit_prompt_topology(workflow_id, node_id, Some(parent))
            .await?)
    }

    /// Deletes a prompt-only node, reconnecting children to its sole parent, with metadata Undo.
    ///
    /// The original identity remains reserved for Undo; no native history or files are changed.
    ///
    /// # Errors
    ///
    /// Refuses unrooted graphs, multiple source parents, native/run/workspace references
    /// anywhere in the subtree, or storage failures.
    pub async fn delete_prompt_node(
        &mut self,
        workflow_id: i64,
        node_id: i64,
    ) -> Result<WorkflowGraph, StorageError> {
        Ok(self
            .edit_prompt_topology(workflow_id, node_id, None)
            .await?)
    }

    async fn edit_prompt_topology(
        &self,
        workflow_id: i64,
        node_id: i64,
        parent: Option<&NodeParent>,
    ) -> anyhow::Result<WorkflowGraph> {
        self.ensure_dependency_operations_clear().await?;
        let mut connection = self.database.connect()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .await?;
        let result = async {
            let graph = dependencies::load_graph(&tx, workflow_id).await?;
            dependencies::require_rooted(&graph)?;
            anyhow::ensure!(
                graph.nodes.iter().any(|node| node.id == node_id),
                "node not found in this workflow"
            );
            let incoming = parents(&graph, node_id);
            anyhow::ensure!(
                incoming.len() == 1,
                "prompt topology edits require exactly one source parent"
            );
            ensure_prompt_subtree(&tx, &graph, &[node_id]).await?;
            if let Some(parent) = parent {
                dependencies::validate_parent(&tx, workflow_id, Some(parent)).await?;
                if incoming == [parent.clone()] {
                    return Ok(graph);
                }
            }
            let mut targets = vec![node_id];
            if parent.is_none() {
                targets.extend(children(&graph, node_id));
            }
            targets.sort_unstable();
            let before = observe(&graph, node_id, &targets)?;
            let mut after = before.clone();
            if let Some(parent) = parent {
                after
                    .attachments
                    .iter_mut()
                    .find(|a| a.node_id == node_id)
                    .context("missing source attachment")?
                    .parents = vec![parent.clone()];
            } else {
                after.node = None;
                after.attachments.retain(|a| a.node_id != node_id);
                for attachment in &mut after.attachments {
                    attachment
                        .parents
                        .retain(|p| *p != NodeParent::Node { node_id });
                    if !attachment.parents.contains(&incoming[0]) {
                        attachment.parents.push(incoming[0].clone());
                    }
                    sort_parents(&mut attachment.parents);
                }
            }
            let change = Change {
                workflow_id,
                node_id,
                before,
                after,
            };
            apply(&tx, &change, &Direction::Redo).await?;
            let edit = if parent.is_some() {
                Edit::MoveNode { change }
            } else {
                Edit::DeleteNode { change }
            };
            append(&tx, &graph.workflow.project_path, &edit).await?;
            dependencies::load_graph(&tx, workflow_id).await
        }
        .await;
        finish(tx, result).await
    }
}

fn sort_parents(parents: &mut [NodeParent]) {
    parents.sort_by_key(|p| match p {
        NodeParent::WorkflowBase {} => (0, 0),
        NodeParent::Node { node_id } => (1, *node_id),
    });
}

fn parents(graph: &WorkflowGraph, node_id: i64) -> Vec<NodeParent> {
    let mut result = Vec::new();
    if graph.base_nodes.contains(&node_id) {
        result.push(NodeParent::WorkflowBase {});
    }
    result.extend(
        graph
            .connections
            .iter()
            .filter(|c| c.target_id == node_id)
            .map(|c| NodeParent::Node {
                node_id: c.source_id,
            }),
    );
    sort_parents(&mut result);
    result
}

fn children(graph: &WorkflowGraph, node_id: i64) -> Vec<i64> {
    graph
        .connections
        .iter()
        .filter(|c| c.source_id == node_id)
        .map(|c| c.target_id)
        .collect()
}

fn observe(graph: &WorkflowGraph, node_id: i64, targets: &[i64]) -> anyhow::Result<State> {
    let node = graph.nodes.iter().find(|n| n.id == node_id).cloned();
    let mut attachments = Vec::new();
    for id in targets {
        if *id == node_id && node.is_none() {
            continue;
        }
        anyhow::ensure!(
            graph.nodes.iter().any(|n| n.id == *id),
            "affected child no longer exists"
        );
        attachments.push(Attachment {
            node_id: *id,
            parents: parents(graph, *id),
        });
    }
    Ok(State { node, attachments })
}

async fn ensure_prompt_subtree(
    tx: &turso::Connection,
    graph: &WorkflowGraph,
    roots: &[i64],
) -> anyhow::Result<()> {
    let mut pending = roots.to_vec();
    let mut visited = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !visited.insert(id) {
            continue;
        }
        pending.extend(children(graph, id));
        let mut rows = tx
            .query(
                "SELECT node_id FROM node_changes WHERE node_id = ?1
             UNION ALL SELECT node_id FROM node_change_creations WHERE node_id = ?1
             UNION ALL SELECT node_id FROM node_workspaces WHERE node_id = ?1
             UNION ALL SELECT node_id FROM agent_runs WHERE node_id = ?1
             UNION ALL SELECT node_id FROM deleted_nodes WHERE node_id = ?1",
                [id],
            )
            .await?;
        anyhow::ensure!(
            rows.next().await?.is_none(),
            "node {id} has native/workspace/run references; use the explicit native editing path"
        );
    }
    Ok(())
}

pub(super) async fn apply(
    tx: &turso::Connection,
    change: &Change,
    direction: &Direction,
) -> anyhow::Result<()> {
    let (expected, replacement) = match direction {
        Direction::Undo => (&change.after, &change.before),
        Direction::Redo => (&change.before, &change.after),
    };
    let graph = dependencies::load_graph(tx, change.workflow_id).await?;
    dependencies::require_rooted(&graph)?;
    let mut targets: Vec<_> = change
        .before
        .attachments
        .iter()
        .map(|a| a.node_id)
        .collect();
    targets.sort_unstable();
    anyhow::ensure!(
        observe(&graph, change.node_id, &targets)? == *expected,
        "prompt node or dependencies changed; refresh before undo/redo"
    );
    // Catch new outgoing edges not covered by the saved local attachment set.
    let expected_children: BTreeSet<_> = expected
        .attachments
        .iter()
        .filter(|a| {
            a.parents.contains(&NodeParent::Node {
                node_id: change.node_id,
            })
        })
        .map(|a| a.node_id)
        .collect();
    if change.before.attachments.len() > 1 || change.after.node.is_none() {
        anyhow::ensure!(
            children(&graph, change.node_id)
                .into_iter()
                .collect::<BTreeSet<_>>()
                == expected_children,
            "node's children changed; cannot partially restore deletion"
        );
    }
    ensure_prompt_subtree(tx, &graph, &targets).await?;
    for id in &targets {
        tx.execute(
            "DELETE FROM connections WHERE workflow_id = ?1 AND target_id = ?2",
            [change.workflow_id, *id],
        )
        .await?;
        tx.execute(
            "DELETE FROM workflow_base_nodes WHERE workflow_id = ?1 AND node_id = ?2",
            [change.workflow_id, *id],
        )
        .await?;
    }
    match (&expected.node, &replacement.node) {
        (None, Some(node)) => insert_node(tx, node).await?,
        (Some(_), None) => {
            tx.execute(
                "DELETE FROM nodes WHERE workflow_id = ?1 AND id = ?2",
                [change.workflow_id, change.node_id],
            )
            .await?;
        }
        _ => {}
    }
    for attachment in &replacement.attachments {
        for parent in &attachment.parents {
            if let NodeParent::Node { node_id } = parent {
                anyhow::ensure!(
                    graph.nodes.iter().any(|n| n.id == *node_id)
                        || replacement.node.as_ref().is_some_and(|n| n.id == *node_id),
                    "replacement parent no longer exists in the active workflow"
                );
            }
            match parent {
                NodeParent::WorkflowBase {} => {
                    dependencies::attach(tx, change.workflow_id, attachment.node_id, parent)
                        .await?;
                }
                NodeParent::Node { node_id } => {
                    insert_connection(
                        tx,
                        &Connection {
                            workflow_id: change.workflow_id,
                            source_id: *node_id,
                            target_id: attachment.node_id,
                        },
                    )
                    .await?;
                }
            }
        }
    }
    let result = dependencies::load_graph(tx, change.workflow_id).await?;
    dependencies::require_rooted(&result)?;
    anyhow::ensure!(
        observe(&result, change.node_id, &targets)? == *replacement,
        "prompt topology edit did not produce the expected state"
    );
    Ok(())
}

#[cfg(test)]
mod tests;
