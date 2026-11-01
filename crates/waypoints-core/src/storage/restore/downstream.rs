//! Confirmed same-workflow restoration with durable, leaf-first checkpoints.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use super::{RestoreImpact, RestoreOperation, RestoreState};
use crate::jj::{self, ChangeState, Revision};
use crate::storage::dependencies;
use crate::storage::reparent::{graph_descendants, verify_node_ancestry};
use crate::storage::{Storage, StorageError, WorkflowGraph};
use crate::workspace::{WorkspaceAssociation, WorkspaceState};

// Keep a single journal response within the RPC frame, including completion fields.
const MAX_RECORD_BYTES: usize = 900 * 1024;

/// Explicit acceptance of code rebasing, stale checkouts and invalidated sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RestoreRebaseConfirmation {
    /// Restore source code, rebase descendants, and require fresh affected sessions.
    RestoreAndRebase,
}

/// Affected task history at this restore, not a claim that its work was rerun.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RestoreAffectedNode {
    /// Saved node identity.
    pub node_id: i64,
    /// Unique original revision, or none for a prompt-only task.
    pub before: Option<Revision>,
    /// Original code parents, including unchanged boundary inputs.
    pub before_parent_commit_ids: Vec<String>,
    /// Original optional allocation; no workspace is silently created.
    pub workspace: Option<WorkspaceAssociation>,
    /// Exact checkpoint before any ancestor rewrite, including tracked local edits.
    pub preserved: Option<Revision>,
    /// Current observed result, not an agent result.
    pub after: Option<Revision>,
    /// Whether the checkout differed at the last observation; not a live status query.
    pub needs_workspace_update: bool,
}

/// Durable checked scope; all old sessions in it are barred at reservation time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RestoreDownstream {
    /// Restore journal watermark explicitly confirmed by the caller.
    pub preview_restore_watermark: i64,
    /// Unchanged graph, compared again on completion and recovery.
    pub graph: WorkflowGraph,
    /// Affected nodes in leaf-first checkpoint order, including prompt-only tasks.
    pub nodes: Vec<RestoreAffectedNode>,
    /// Explicit reminder that native rebase does not redo an agent's task.
    pub context_notice: String,
}

impl Storage {
    /// Applies an explicitly confirmed restore against a fresh, equal impact report.
    ///
    /// Checkpoints allocated workspaces leaf-first, then restores the source and
    /// lets native jj rebase descendants. Descendant physical checkouts are not
    /// updated. All old sessions on every affected task require a fresh start.
    /// Graph/history and old run records are unchanged, even for clean rebases.
    ///
    /// # Errors
    ///
    /// Refuses missing confirmation, changed previews, non-rooted/misaligned or
    /// cross-workflow/unmanaged work, immutable/divergent changes, unsafe/shared
    /// workspaces, active runs and incomplete allocations. Any uncertainty after
    /// reservation keeps the global guard and checkpoints for explicit recovery.
    pub async fn restore_node_with_descendants(
        &mut self,
        preview: &RestoreImpact,
        confirmation: Option<RestoreRebaseConfirmation>,
    ) -> Result<RestoreOperation, StorageError> {
        Ok(tokio::time::timeout(super::RESTORE_TIMEOUT, async {
            anyhow::ensure!(confirmation == Some(RestoreRebaseConfirmation::RestoreAndRebase),
                "downstream restore requires explicit restore_and_rebase confirmation");
            self.ensure_dependency_operations_clear().await?;
            self.ensure_no_restore_writers().await?;
            let fresh = self.node_restore_impact(
                preview.workflow_id, preview.node_id, &preview.selected.commit_id,
            ).await?;
            anyhow::ensure!(*preview == fresh, "restore preview changed; refresh and confirm again");
            dependencies::require_rooted(&fresh.graph)?;
            anyhow::ensure!(fresh.parents_match, "selected revision has different exact parents");
            let expected: BTreeSet<_> = fresh.tasks.iter().map(|task| task.node.id).collect();
            anyhow::ensure!(fresh.tasks.iter().all(|task| task.node.workflow_id == fresh.workflow_id)
                && expected == graph_descendants(&fresh.graph, fresh.node_id),
                "restore only supports this workflow's saved subtree; cross-workflow/native graph mismatch");
            anyhow::ensure!(fresh.changes.iter().all(|change| !change.immutable && change.node_ids.len() == 1),
                "restore refuses immutable, unmanaged or ambiguously owned native descendants");
            let mut pending: BTreeMap<_, _> = fresh.tasks.iter().map(|task| {
                let change = fresh.changes.iter().find(|change| change.node_ids == [task.node.id]);
                (task.node.id, RestoreAffectedNode {
                    node_id: task.node.id, before: change.map(|change| change.revision.clone()),
                    before_parent_commit_ids: change.map_or_else(Vec::new, |change| change.parent_commit_ids.clone()),
                    workspace: task.allocation.as_ref().map(|a| a.workspace.clone()),
                    preserved: None, after: None, needs_workspace_update: false,
                })
            }).collect();
            anyhow::ensure!(fresh.tasks.iter().all(|task|
                task.association.is_none() || task.native_affected),
                "saved downstream association is not a native descendant");
            let mut nodes = Vec::new();
            while !pending.is_empty() {
                let leaf = *pending.keys().find(|id| !fresh.graph.connections.iter().any(|edge|
                    edge.source_id == **id && pending.contains_key(&edge.target_id)))
                    .context("saved graph contains a cycle")?;
                nodes.push(pending.remove(&leaf).context("missing checkpoint task")?);
            }
            let source = nodes.iter().find(|node| node.node_id == fresh.node_id)
                .context("source missing from restore scope")?;
            let workspace = source.workspace.clone().context("source requires a dedicated workspace")?;
            let mut operation = RestoreOperation {
                id: 0, run_id: None, workflow_id: fresh.workflow_id, node_id: fresh.node_id,
                workspace, source: fresh.selected, before: fresh.current,
                preserved: None, after: None, native_operation: fresh.native_operation.clone(),
                session_barrier: 0, state: RestoreState::Unresolved,
                downstream: Some(RestoreDownstream {
                    preview_restore_watermark: fresh.restore_watermark,
                    graph: fresh.graph, nodes,
                    context_notice: "Code was not rerun by agents. Every pre-existing session on an affected task requires a fresh start; inspect rebased code and conflicts even when the rebase is clean.".into(),
                }),
            };
            self.ensure_restore_allocations_clear().await?;
            self.observe_restore_subtree(&mut operation, CheckoutRequirement::Fresh).await?;
            ensure_response_capacity(&operation)?;
            require_operation(&operation.workspace.repository_path, &fresh.native_operation).await?;
            self.reserve_restore(&mut operation).await?;
            let result = async {
                let mut native_operation = fresh.native_operation;
                let count = operation.downstream.as_ref().context("missing scope")?.nodes.len();
                for index in 0..count {
                    require_operation(&operation.workspace.repository_path, &native_operation).await?;
                    let node = &operation.downstream.as_ref().context("missing scope")?.nodes[index];
                    if let Some(before) = &node.before {
                        let preserved = if let Some(workspace) = &node.workspace {
                            jj::snapshot_workspace(workspace, before).await?
                        } else {
                            before.clone()
                        };
                        if node.node_id == operation.node_id {
                            operation.preserved = Some(preserved.clone());
                        }
                        operation.downstream.as_mut().context("missing scope")?.nodes[index].preserved = Some(preserved);
                        self.save_restore(&operation).await?;
                        native_operation = checked_transition(
                            &operation.workspace.repository_path, &native_operation,
                        ).await?;
                    }
                }
                self.observe_restore_subtree(&mut operation, CheckoutRequirement::Recorded).await?;
                require_operation(&operation.workspace.repository_path, &native_operation).await?;
                self.save_restore(&operation).await?;
                let preserved = operation.preserved.as_ref().context("source checkpoint missing")?;
                operation.after = Some(jj::restore_workspace(
                    &operation.workspace, &operation.source, preserved,
                ).await?);
                native_operation = checked_transition(
                    &operation.workspace.repository_path, &native_operation,
                ).await?;
                self.observe_restore_subtree(&mut operation, CheckoutRequirement::Recorded).await?;
                require_operation(&operation.workspace.repository_path, &native_operation).await?;
                operation.state = RestoreState::Completed;
                self.save_restore(&operation).await?;
                Ok::<_, anyhow::Error>(())
            }.await;
            result.with_context(|| format!(
                "restore operation {} may be incomplete; inspect node restores and reconcile all recorded workspaces before explicit recovery; never retry blindly",
                operation.id,
            ))?;
            Ok::<_, anyhow::Error>(operation)
        }).await.context("downstream restore timed out; inspect the journal before further work")??)
    }

    async fn ensure_restore_allocations_clear(&self) -> anyhow::Result<()> {
        let connection = self.database.connect()?;
        let mut rows = connection
            .query(
                "SELECT node_id FROM node_change_creations UNION ALL
             SELECT node_id FROM node_workspaces WHERE ready = 0 LIMIT 1",
                (),
            )
            .await?;
        anyhow::ensure!(
            rows.next().await?.is_none(),
            "resolve incomplete native allocations before downstream restoration"
        );
        Ok(())
    }

    pub(super) async fn recover_downstream_restore(
        &self,
        mut operation: RestoreOperation,
    ) -> anyhow::Result<RestoreOperation> {
        let native_operation = jj::operation_id(&operation.workspace.repository_path).await?;
        self.ensure_restore_allocations_clear().await?;
        self.observe_restore_subtree(&mut operation, CheckoutRequirement::Fresh)
            .await?;
        require_operation(&operation.workspace.repository_path, &native_operation).await?;
        operation.after = operation
            .downstream
            .as_ref()
            .context("missing scope")?
            .nodes
            .iter()
            .find(|node| node.node_id == operation.node_id)
            .and_then(|node| node.after.clone());
        operation.state = RestoreState::Recovered;
        self.save_restore(&operation).await?;
        Ok(operation)
    }

    async fn observe_restore_subtree(
        &self,
        operation: &mut RestoreOperation,
        checkouts: CheckoutRequirement,
    ) -> anyhow::Result<()> {
        let root = &operation.workspace.repository_path;
        let scope = operation
            .downstream
            .as_mut()
            .context("missing downstream scope")?;
        anyhow::ensure!(
            self.workflow_graph(operation.workflow_id).await? == scope.graph,
            "saved graph changed since confirmation"
        );
        let before_ids: BTreeSet<_> = scope
            .nodes
            .iter()
            .filter_map(|node| node.before.as_ref().map(|rev| rev.commit_id.clone()))
            .collect();
        let mut current = BTreeMap::new();
        for node in &mut scope.nodes {
            let refreshed = self
                .refresh_node(operation.workflow_id, node.node_id)
                .await?;
            match (&node.before, refreshed.state) {
                (None, ChangeState::Unassociated) => {
                    anyhow::ensure!(node.workspace.is_none(), "prompt-only task has a workspace");
                }
                (Some(before), ChangeState::Current { revision }) => {
                    anyhow::ensure!(
                        revision.change_id == before.change_id
                            && revision.description == before.description
                            && refreshed
                                .association
                                .as_ref()
                                .is_some_and(|a| a.repository_path == *root),
                        "affected native association or description changed"
                    );
                    current.insert(node.node_id, revision);
                }
                _ => {
                    anyhow::bail!("affected task has missing, divergent or mismatched native work")
                }
            }
            let observed = self
                .node_workspace(operation.workflow_id, node.node_id)
                .await?;
            anyhow::ensure!(
                observed.workspace == node.workspace,
                "workspace allocation changed"
            );
            if node.workspace.is_some() {
                self.require_ready_workspace(node.node_id).await?;
                anyhow::ensure!(
                    matches!(observed.state, WorkspaceState::Ready { .. }),
                    "workspace registry or directory is mismatched"
                );
            }
        }
        let source = current.get(&operation.node_id).context("source missing")?;
        let actual: BTreeSet<_> = jj::descendants(root, source)
            .await?
            .into_iter()
            .map(|r| r.commit_id)
            .collect();
        let expected: BTreeSet<_> = current.values().map(|r| r.commit_id.clone()).collect();
        anyhow::ensure!(
            actual == expected,
            "native descendants changed or include unmanaged/other-workflow work"
        );
        let mut parents_by_node = current.clone();
        for edge in scope
            .graph
            .connections
            .iter()
            .filter(|edge| current.contains_key(&edge.target_id))
        {
            if let std::collections::btree_map::Entry::Vacant(entry) =
                parents_by_node.entry(edge.source_id)
            {
                let parent = self
                    .refresh_node(operation.workflow_id, edge.source_id)
                    .await?;
                anyhow::ensure!(
                    parent
                        .association
                        .as_ref()
                        .is_some_and(|a| a.repository_path == *root),
                    "boundary parent repository changed"
                );
                let ChangeState::Current { revision } = parent.state else {
                    anyhow::bail!("boundary parent is missing or divergent");
                };
                entry.insert(revision);
            }
        }
        for node in &mut scope.nodes {
            if let Some(revision) = current.get(&node.node_id) {
                verify_node_ancestry(root, &scope.graph, node.node_id, revision, &parents_by_node)
                    .await?;
                let parents = jj::parent_commit_ids(root, &revision.commit_id).await?;
                let boundary: BTreeSet<_> = node
                    .before_parent_commit_ids
                    .iter()
                    .filter(|id| !before_ids.contains(*id))
                    .collect();
                let current_boundary: BTreeSet<_> = parents
                    .iter()
                    .filter(|id| !expected.contains(*id))
                    .collect();
                anyhow::ensure!(
                    boundary == current_boundary,
                    "unaffected boundary parents changed"
                );
                node.after = Some(revision.clone());
                if let Some(workspace) = &node.workspace {
                    node.needs_workspace_update =
                        jj::checked_out_revision(&workspace.directory).await? != *revision;
                    anyhow::ensure!(
                        !matches!(checkouts, CheckoutRequirement::Fresh)
                            || !node.needs_workspace_update,
                        "affected checkout is stale; inspect and explicitly reconcile its workspace first"
                    );
                }
            }
        }
        for workspace in jj::workspaces(root).await? {
            if current
                .values()
                .any(|rev| rev.change_id == workspace.change_id)
            {
                anyhow::ensure!(
                    scope
                        .nodes
                        .iter()
                        .filter_map(|node| node.workspace.as_ref())
                        .any(|saved| saved.name == workspace.name
                            && Some(&saved.directory) == workspace.directory.as_ref()
                            && saved.change_id == workspace.change_id),
                    "another or unmanaged workspace edits an affected change"
                );
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum CheckoutRequirement {
    Fresh,
    Recorded,
}

async fn require_operation(root: &Path, expected: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        jj::operation_id(root).await? == expected,
        "native repository changed; inspect the reserved operation before further work"
    );
    Ok(())
}

async fn checked_transition(root: &Path, before: &str) -> anyhow::Result<String> {
    let after = jj::operation_id(root).await?;
    anyhow::ensure!(
        after == before || jj::operation_parent_ids(root).await? == [before],
        "unexpected concurrent native operation; reservation retained"
    );
    Ok(after)
}

fn ensure_response_capacity(operation: &RestoreOperation) -> anyhow::Result<()> {
    let mut completed = operation.clone();
    completed.id = i64::MAX;
    completed.session_barrier = i64::MAX;
    completed.preserved = Some(completed.before.clone());
    completed.after = Some(completed.before.clone());
    for revision in [&mut completed.preserved, &mut completed.after]
        .into_iter()
        .flatten()
    {
        revision.conflicted = false;
    }
    for node in &mut completed
        .downstream
        .as_mut()
        .context("missing scope")?
        .nodes
    {
        node.preserved = node.before.clone();
        node.after = node.before.clone();
        for revision in [&mut node.preserved, &mut node.after].into_iter().flatten() {
            revision.conflicted = false; // JSON false occupies more bytes than true.
        }
    }
    anyhow::ensure!(
        serde_json::to_vec(&completed)?.len() <= MAX_RECORD_BYTES,
        "completed restore record would exceed the response limit; no reservation created"
    );
    Ok(())
}
