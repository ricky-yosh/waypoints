//! Explicit graph roots, node dependencies, and native creation preflight.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::PathBuf;

use anyhow::Context;

use super::{Connection, Storage, StorageError, WorkflowGraph, decode_node, decode_workflow};
use crate::jj::{self, ChangeState, Revision};
use crate::workflow::{GraphConnectivity, NodeParent};

impl Storage {
    pub(super) async fn initialize_dependencies(&self) -> anyhow::Result<()> {
        self.database
            .connect()?
            .execute(
                "CREATE TABLE IF NOT EXISTS workflow_base_nodes (
                workflow_id INTEGER NOT NULL,
                node_id INTEGER PRIMARY KEY,
                FOREIGN KEY (workflow_id, node_id) REFERENCES nodes(workflow_id, id)
            )",
                (),
            )
            .await?;
        Ok(())
    }

    /// Explicitly attaches every legacy graph root to the selected workflow base.
    ///
    /// This metadata-only adoption refuses native-associated graphs: their native
    /// dependency reconciliation requires a separate checked operation.
    ///
    /// # Errors
    ///
    /// Returns an error for no base, incomplete/extra roots, existing adoption,
    /// native references, or storage failure. No partial adoption is saved.
    pub async fn adopt_workflow_roots(
        &mut self,
        workflow_id: i64,
        base_nodes: &[i64],
    ) -> Result<WorkflowGraph, StorageError> {
        self.ensure_dependency_operations_clear().await?;
        Ok(async {
            let mut connection = self.database.connect()?;
            let tx = connection.transaction_with_behavior(turso::transaction::TransactionBehavior::Immediate).await?;
            let result = async {
                let graph = load_graph(&tx, workflow_id).await?;
                anyhow::ensure!(graph.workflow.base.is_some(), "select a workflow base first");
                anyhow::ensure!(matches!(graph.connectivity, GraphConnectivity::NeedsAdoption { .. }), "workflow does not need root adoption");
                let selected: BTreeSet<_> = base_nodes.iter().copied().collect();
                anyhow::ensure!(selected.len() == base_nodes.len(), "duplicate base nodes are not allowed");
                let targets: HashSet<_> = graph.connections.iter().map(|edge| edge.target_id).collect();
                let roots: BTreeSet<_> = graph.nodes.iter().filter(|node| !targets.contains(&node.id)).map(|node| node.id).collect();
                anyhow::ensure!(selected == roots, "explicitly select all and only existing graph roots: {roots:?}");
                let mut rows = tx.query(
                    "SELECT n.id FROM nodes n WHERE n.workflow_id = ?1 AND (
                       EXISTS (SELECT 1 FROM node_changes WHERE node_id = n.id) OR
                       EXISTS (SELECT 1 FROM node_change_creations WHERE node_id = n.id) OR
                       EXISTS (SELECT 1 FROM node_workspaces WHERE node_id = n.id) OR
                       EXISTS (SELECT 1 FROM agent_runs WHERE node_id = n.id)) LIMIT 1",
                    [workflow_id],
                ).await?;
                anyhow::ensure!(rows.next().await?.is_none(), "native-associated workflows require native dependency reconciliation before adoption; existing work is unchanged");
                drop(rows);
                for id in base_nodes {
                    tx.execute(
                        "INSERT INTO workflow_base_nodes (workflow_id, node_id) VALUES (?1, ?2)",
                        [workflow_id, *id],
                    ).await?;
                }
                let graph = load_graph(&tx, workflow_id).await?;
                require_rooted(&graph)?;
                Ok(graph)
            }.await;
            match result {
                Ok(graph) => { tx.commit().await?; Ok(graph) }
                Err(error) => { tx.rollback().await?; Err(error) }
            }
        }.await?)
    }

    pub(super) async fn dependency_revisions(
        &self,
        workflow_id: i64,
        node_id: i64,
    ) -> anyhow::Result<Option<(PathBuf, Vec<Revision>)>> {
        let graph = self.workflow_graph(workflow_id).await?;
        let Some(base) = &graph.workflow.base else {
            return Ok(None);
        };
        require_rooted(&graph)?;
        anyhow::ensure!(
            graph.nodes.iter().any(|node| node.id == node_id),
            "node not found in workflow"
        );
        let root = jj::repository_root(&graph.workflow.project_path).await?;
        anyhow::ensure!(
            root == base.repository_path,
            "workflow's native repository root changed"
        );
        let mut parents = Vec::new();
        if graph.base_nodes.contains(&node_id) {
            parents.push(jj::resolve_base(&root, &base.revision.commit_id).await?);
        }
        for edge in graph
            .connections
            .iter()
            .filter(|edge| edge.target_id == node_id)
        {
            self.ensure_node_idle(edge.source_id).await?;
            let upstream = self.refresh_node(workflow_id, edge.source_id).await?;
            match upstream.state {
                ChangeState::Current { revision } => parents.push(revision),
                _ => anyhow::bail!(
                    "upstream node {} needs one current native change before creating dependent code",
                    edge.source_id
                ),
            }
        }
        anyhow::ensure!(!parents.is_empty(), "node has no dependency");
        Ok(Some((root, parents)))
    }

    pub(super) async fn validate_dependency_association(
        &self,
        workflow_id: i64,
        node_id: i64,
        root: &std::path::Path,
        revision: &Revision,
    ) -> anyhow::Result<()> {
        let dependency = self.dependency_revisions(workflow_id, node_id).await?;
        let based = dependency.is_some();
        if let Some((expected_root, parents)) = dependency {
            anyhow::ensure!(
                root == expected_root,
                "dependency repository does not match node repository"
            );
            let mut expected: Vec<_> = parents.into_iter().map(|parent| parent.commit_id).collect();
            expected.sort();
            expected.dedup();
            let mut actual = jj::parent_commit_ids(root, &revision.commit_id).await?;
            actual.sort();
            anyhow::ensure!(
                actual == expected,
                "selected change's native parents do not match the node's saved dependencies; no code was rebased"
            );
        }
        let connection = self.database.connect()?;
        let mut rows = connection
            .query(
                "SELECT c.node_id FROM node_changes c JOIN nodes n ON n.id = c.node_id
             LEFT JOIN workflow_bases b ON b.workflow_id = n.workflow_id
             WHERE c.repository_path = ?1 AND c.change_id = ?2 AND c.node_id <> ?3
               AND NOT EXISTS (SELECT 1 FROM deleted_nodes WHERE node_id = n.id)
               AND (?4 = 1 OR b.workflow_id IS NOT NULL) LIMIT 1",
                turso::params![
                    root.to_str(),
                    revision.change_id.clone(),
                    node_id,
                    i64::from(based)
                ],
            )
            .await?;
        anyhow::ensure!(
            rows.next().await?.is_none(),
            "native change is already associated with another node"
        );
        Ok(())
    }
}

pub(super) async fn load_graph(
    connection: &turso::Connection,
    workflow_id: i64,
) -> anyhow::Result<WorkflowGraph> {
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
    let workflow = decode_workflow(&row)?;
    drop(rows);
    let mut rows = connection
        .query(
            "SELECT id, workflow_id, prompt FROM nodes WHERE workflow_id = ?1
             AND NOT EXISTS (SELECT 1 FROM deleted_nodes WHERE node_id = nodes.id) ORDER BY id",
            [workflow_id],
        )
        .await?;
    let mut nodes = Vec::new();
    while let Some(row) = rows.next().await? {
        nodes.push(decode_node(&row)?);
    }
    drop(rows);
    let mut rows = connection
        .query(
            "SELECT workflow_id, source_id, target_id FROM connections
         WHERE workflow_id = ?1 ORDER BY source_id, target_id",
            [workflow_id],
        )
        .await?;
    let mut connections = Vec::new();
    while let Some(row) = rows.next().await? {
        connections.push(Connection {
            workflow_id: row.get(0)?,
            source_id: row.get(1)?,
            target_id: row.get(2)?,
        });
    }
    drop(rows);
    let mut rows = connection
        .query(
            "SELECT node_id FROM workflow_base_nodes WHERE workflow_id = ?1 ORDER BY node_id",
            [workflow_id],
        )
        .await?;
    let mut base_nodes = Vec::new();
    while let Some(row) = rows.next().await? {
        base_nodes.push(row.get::<i64>(0)?);
    }
    let connectivity = if workflow.base.is_none() {
        GraphConnectivity::Unbased
    } else {
        let mut children: HashMap<i64, Vec<i64>> = HashMap::new();
        for edge in &connections {
            children
                .entry(edge.source_id)
                .or_default()
                .push(edge.target_id);
        }
        let mut reachable: HashSet<_> = base_nodes.iter().copied().collect();
        let mut pending: VecDeque<_> = base_nodes.iter().copied().collect();
        while let Some(parent) = pending.pop_front() {
            for child in children.get(&parent).into_iter().flatten() {
                if reachable.insert(*child) {
                    pending.push_back(*child);
                }
            }
        }
        let missing: Vec<_> = nodes
            .iter()
            .filter(|node| !reachable.contains(&node.id))
            .map(|node| node.id)
            .collect();
        if missing.is_empty() {
            GraphConnectivity::Rooted
        } else {
            GraphConnectivity::NeedsAdoption { node_ids: missing }
        }
    };
    drop(rows);
    let mut rows = connection
        .query(
            "SELECT id FROM dependency_operations WHERE workflow_id = ?1 AND active = 1",
            [workflow_id],
        )
        .await?;
    let unresolved_dependency_operation = rows.next().await?.map(|row| row.get(0)).transpose()?;
    Ok(WorkflowGraph {
        workflow,
        nodes,
        connections,
        base_nodes,
        connectivity,
        unresolved_dependency_operation,
    })
}

pub(super) fn require_rooted(graph: &WorkflowGraph) -> anyhow::Result<()> {
    anyhow::ensure!(
        matches!(graph.connectivity, GraphConnectivity::Rooted),
        "workflow must have a base and explicit paths to it; inspect graph and adopt legacy roots first"
    );
    Ok(())
}

pub(super) async fn validate_parent(
    tx: &turso::Connection,
    workflow_id: i64,
    parent: Option<&NodeParent>,
) -> anyhow::Result<()> {
    validate_parents(
        tx,
        workflow_id,
        parent.map_or(&[][..], std::slice::from_ref),
    )
    .await
}

pub(super) async fn validate_parents(
    tx: &turso::Connection,
    workflow_id: i64,
    parents: &[NodeParent],
) -> anyhow::Result<()> {
    let graph = load_graph(tx, workflow_id).await?;
    match (graph.workflow.base.is_some(), parents.is_empty()) {
        (false, true) => Ok(()),
        (false, false) => anyhow::bail!("select a workflow base before attaching nodes"),
        (true, true) => {
            anyhow::bail!("based workflows require an explicit dependency: --from-base or --parent")
        }
        (true, false) => {
            require_rooted(&graph)?;
            let nodes: HashSet<_> = graph.nodes.iter().map(|node| node.id).collect();
            let mut selected = HashSet::new();
            for parent in parents {
                match parent {
                    NodeParent::Node { node_id } => {
                        anyhow::ensure!(
                            selected.insert(*node_id),
                            "duplicate node dependencies are not allowed"
                        );
                        anyhow::ensure!(
                            nodes.contains(node_id),
                            "upstream node {node_id} not found in this workflow"
                        );
                    }
                    NodeParent::WorkflowBase {} => anyhow::ensure!(
                        parents.len() == 1,
                        "the workflow base cannot be combined with upstream nodes"
                    ),
                }
            }
            Ok(())
        }
    }
}

pub(super) async fn attach(
    tx: &turso::Connection,
    workflow_id: i64,
    node_id: i64,
    parent: &NodeParent,
) -> anyhow::Result<()> {
    match parent {
        NodeParent::WorkflowBase {} => {
            tx.execute(
                "INSERT INTO workflow_base_nodes (workflow_id, node_id) VALUES (?1, ?2)",
                [workflow_id, node_id],
            )
            .await?;
        }
        NodeParent::Node { node_id: upstream } => {
            tx.execute(
                "INSERT INTO connections (workflow_id, source_id, target_id) VALUES (?1, ?2, ?3)",
                [workflow_id, *upstream, node_id],
            )
            .await?;
        }
    }
    Ok(())
}

pub(super) async fn detach_created_node(
    tx: &turso::Connection,
    workflow_id: i64,
    node_id: i64,
    parents: &[NodeParent],
) -> anyhow::Result<()> {
    let graph = load_graph(tx, workflow_id).await?;
    let incoming: Vec<_> = graph
        .connections
        .iter()
        .filter(|edge| edge.target_id == node_id)
        .map(|edge| edge.source_id)
        .collect();
    let is_base = graph.base_nodes.contains(&node_id);
    let mut expected: Vec<_> = parents
        .iter()
        .filter_map(|parent| match parent {
            NodeParent::Node { node_id } => Some(*node_id),
            NodeParent::WorkflowBase {} => None,
        })
        .collect();
    expected.sort();
    let matches = is_base == parents.contains(&NodeParent::WorkflowBase {}) && incoming == expected;
    anyhow::ensure!(
        matches,
        "node dependencies changed; cannot partially undo node creation"
    );
    tx.execute(
        "DELETE FROM workflow_base_nodes WHERE workflow_id = ?1 AND node_id = ?2",
        [workflow_id, node_id],
    )
    .await?;
    tx.execute(
        "DELETE FROM connections WHERE workflow_id = ?1 AND target_id = ?2",
        [workflow_id, node_id],
    )
    .await?;
    Ok(())
}

pub(super) async fn require_legacy_connections(
    tx: &turso::Connection,
    workflow_id: i64,
) -> anyhow::Result<()> {
    let graph = load_graph(tx, workflow_id).await?;
    anyhow::ensure!(
        graph.workflow.base.is_none(),
        "metadata-only connection edits are unavailable for based workflows; choose --parent when creating a node or use node reparent for native dependency editing"
    );
    Ok(())
}
