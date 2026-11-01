//! Conservative restore impact from native ancestry and saved graph/session references.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::jj::{self, ChangeAssociation, Revision};
use crate::storage::{Node, Storage, StorageError, WorkflowGraph};
use crate::workspace::WorkspaceAssociation;

// Leave room for the JSON-RPC envelope and escaping within the 1 MiB frame.
const MAX_REPORT_BYTES: usize = 900 * 1024;

/// Readonly restore impact, not a conflict simulation, reservation, or permission to apply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RestoreImpact {
    /// Workflow containing the selected node.
    pub workflow_id: i64,
    /// Node whose existing change would receive the selected tree.
    pub node_id: i64,
    /// Canonical repository supplying the native history.
    pub repository_path: PathBuf,
    /// Native operation pinned for ancestry, registry, mutability and tree comparison.
    pub native_operation: String,
    /// Unique current recorded revision of the source change.
    pub current: Revision,
    /// Exact selected revision of that same change.
    pub selected: Revision,
    /// Tree comparison excludes unsnapshotted files, even when false.
    pub recorded_tree_changed: bool,
    /// Whether selection retains the current exact code-parent set.
    pub parents_match: bool,
    /// Source and all visible native descendants, including unmanaged/other-workflow work.
    pub changes: Vec<RestoreImpactChange>,
    /// Saved tasks reached through native identity or transitive saved dependency edges.
    pub tasks: Vec<RestoreImpactTask>,
    /// Native workspaces editing affected commits; physical files are never snapshotted.
    pub workspaces: Vec<RestoreImpactWorkspace>,
    /// Historical session references, not a claim about their actual reasoning context.
    pub sessions: Vec<RestoreImpactSession>,
    /// All database-wide active/interrupted run reservations, including unrelated tasks.
    pub active_run_ids: Vec<i64>,
    /// Known blockers, not an exhaustive apply preflight or a reusable authorization.
    pub observed_blockers: Vec<String>,
    /// Limits that callers must retain when presenting the report.
    pub notes: Vec<String>,
    /// Saved source-workflow graph, including edges, for explicit compare-and-apply.
    pub graph: WorkflowGraph,
    /// Last database-wide restore ID; invalidates confirmations even after a native no-op.
    pub restore_watermark: i64,
}

/// A native revision potentially rewritten if the source tree changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RestoreImpactChange {
    /// Existing revision; conflicts here are existing conflicts, not predicted ones.
    pub revision: Revision,
    /// Existing exact code parents, including all merge inputs.
    pub parent_commit_ids: Vec<String>,
    /// Native immutable policy at the pinned operation.
    pub immutable: bool,
    /// Live saved tasks owning this change; empty means unmanaged native work.
    pub node_ids: Vec<i64>,
}

/// A saved workspace allocation, without claiming its files or registry are current.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RestoreImpactAllocation {
    /// Persisted allocation reference.
    pub workspace: WorkspaceAssociation,
    /// Whether initial allocation completed, not physical checkout freshness.
    pub ready: bool,
}

/// A potentially affected task; graph-only tasks need not have any native code yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RestoreImpactTask {
    /// Saved task, possibly in another workflow sharing the repository.
    pub node: Node,
    /// Saved change identity; a graph edge is not proof of native ancestry.
    pub association: Option<ChangeAssociation>,
    /// Whether that identity occurs in the affected native revision set.
    pub native_affected: bool,
    /// Saved allocation, including incomplete or externally missing allocations.
    pub allocation: Option<RestoreImpactAllocation>,
}

/// Physical checkout observation, never an assertion that files are clean.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
#[non_exhaustive]
pub enum RestoreImpactCheckout {
    /// Working-copy metadata matches the pinned native workspace target.
    CurrentRecorded,
    /// Physical checkout metadata still identifies another exact revision.
    Stale {
        /// Actual recorded physical checkout.
        commit_id: String,
    },
    /// Checkout could not be verified; no repair was attempted.
    Unknown {
        /// Missing directory, unsupported metadata, or inspection diagnostic.
        detail: String,
    },
}

/// A native workspace potentially made stale by rewriting its recorded target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RestoreImpactWorkspace {
    /// Native workspace name, including workspaces not allocated by Waypoints.
    pub name: String,
    /// Native recorded directory, which may be unavailable.
    pub directory: Option<PathBuf>,
    /// Exact affected native target.
    pub revision: Revision,
    /// Best-effort readonly physical metadata observation.
    pub checkout: RestoreImpactCheckout,
}

/// Runs sharing one recorded session on an affected task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RestoreImpactSession {
    /// Task owning these historical run records.
    pub node_id: i64,
    /// Absent when no session was established or persisted.
    pub session_id: Option<String>,
    /// Matching run IDs, oldest first; no prompt/transcript is replayed.
    pub run_ids: Vec<i64>,
    /// Whether this is the source node, where applying restore requires a fresh session.
    pub source_node: bool,
}

impl Storage {
    /// Previews same-change restore impact without writing jj, files, sessions, or journals.
    ///
    /// Includes native merge/external descendants and graph-only downstream tasks.
    /// Native facts are pinned and the live operation is checked again before return.
    /// The report remains informational: local files/context and future rebase
    /// conflicts are not simulated. Existing restore commands still refuse descendants.
    ///
    /// # Errors
    ///
    /// Refuses invalid membership/selectors, a missing/divergent source, foreign
    /// selection, changed repository state, oversized output, timeout or unreadable
    /// native/database facts. Workspace inspection failures are explicit unknowns.
    pub async fn node_restore_impact(
        &self,
        workflow_id: i64,
        node_id: i64,
        commit_id: &str,
    ) -> Result<RestoreImpact, StorageError> {
        jj::validate_commit_id(commit_id)?;
        Ok(tokio::time::timeout(super::RESTORE_TIMEOUT, async {
            self.node(workflow_id, node_id).await?;
            let association = self.node_change_association(node_id).await?
                .context("restore impact requires an associated native change")?;
            let project = self.project_for_node(workflow_id, node_id).await?;
            let root = jj::repository_root(&project).await?;
            anyhow::ensure!(root == association.repository_path, "native repository root changed");
            let operation = jj::operation_id(&root).await?;
            let heads = jj::evolution_heads(&root, &association.change_id, &operation).await?;
            anyhow::ensure!(heads.len() == 1,
                "restore impact requires one current revision; source change is missing or divergent");
            let current = heads.into_iter().next().context("missing source revision")?;
            let mut selections = jj::impact_revisions(&root, &operation, commit_id).await?;
            anyhow::ensure!(selections.len() == 1, "selected revision is unavailable");
            let selected = selections.remove(0);
            anyhow::ensure!(selected.revision.change_id == association.change_id,
                "selected revision belongs to another change; cross-change restoration is not supported");
            let native = jj::impact_revisions(
                &root, &operation, &format!("{}::", current.commit_id),
            ).await?;
            let source = native.iter().find(|entry| entry.revision.commit_id == current.commit_id)
                .context("source missing from native impact")?;
            let parents_match = source.parent_commit_ids == selected.parent_commit_ids;
            let changed = jj::impact_tree_changed(&root, &operation, &current, &selected.revision).await?;
            let tasks = self.restore_impact_tasks(node_id, &root, &native).await?;
            let mut changes: Vec<_> = native.into_iter().map(|entry| RestoreImpactChange {
                node_ids: tasks.iter().filter(|task| task.native_affected &&
                    task.association.as_ref().is_some_and(|a| a.change_id == entry.revision.change_id))
                    .map(|task| task.node.id).collect(),
                revision: entry.revision, parent_commit_ids: entry.parent_commit_ids,
                immutable: entry.immutable,
            }).collect();
            changes.sort_by(|a, b| a.revision.commit_id.cmp(&b.revision.commit_id));
            let mut workspaces = Vec::new();
            for workspace in jj::impact_workspaces(&root, &operation).await? {
                if let Some(change) = changes.iter().find(|entry|
                    entry.revision.commit_id == workspace.commit_id)
                {
                    let checkout = observe_checkout(&workspace).await;
                    workspaces.push(RestoreImpactWorkspace {
                        name: workspace.name, directory: workspace.directory,
                        revision: change.revision.clone(), checkout,
                    });
                }
            }
            workspaces.sort_by(|a, b| a.name.cmp(&b.name));
            let sessions = self.restore_impact_sessions(node_id, &tasks).await?;
            let (active_run_ids, mut observed_blockers) = self.restore_impact_reservations().await?;
            if !parents_match {
                observed_blockers.push("selected revision has different exact parents; cross-ancestry restore is not supported".into());
            }
            if changes.iter().any(|change| change.immutable) {
                observed_blockers.push("affected native revisions include immutable work".into());
            }
            if changes.len() > 1 || tasks.iter().any(|task| task.node.id != node_id) {
                observed_blockers.push("downstream restore requires explicit preview confirmation and checked same-workflow application; ordinary restore still refuses descendants".into());
            }
            let target = tasks.iter().find(|task| task.node.id == node_id)
                .context("source task missing from restore impact")?;
            match &target.allocation {
                Some(allocation) if allocation.ready => {
                    let saved = &allocation.workspace;
                    if saved.repository_path != root || saved.change_id != current.change_id ||
                        !workspaces.iter().any(|w| w.name == saved.name &&
                            w.directory.as_ref() == Some(&saved.directory) &&
                            matches!(w.checkout, RestoreImpactCheckout::CurrentRecorded))
                    {
                        observed_blockers.push("source workspace is missing, stale or mismatched; no repair attempted".into());
                    }
                    if workspaces.iter().any(|w| w.revision.change_id == current.change_id &&
                        (w.name != saved.name || w.directory.as_ref() != Some(&saved.directory)))
                    {
                        observed_blockers.push("another native workspace edits the source change".into());
                    }
                }
                _ => observed_blockers.push("source requires a completed dedicated workspace allocation".into()),
            }
            anyhow::ensure!(jj::operation_id(&root).await? == operation,
                "native repository changed during restore impact inspection; refresh the preview");
            let report = RestoreImpact {
                workflow_id, node_id, repository_path: root, native_operation: operation,
                current, selected: selected.revision, recorded_tree_changed: changed,
                parents_match, changes, tasks, workspaces, sessions, active_run_ids, observed_blockers,
                graph: self.workflow_graph(workflow_id).await?,
                restore_watermark: self.restore_preview_watermark().await?,
                notes: vec![
                    "Conservative impact only: no restore/rebase was simulated; future conflicts are unknown.".into(),
                    "Unsnapshotted and ignored files are not inspected or backed up; unchanged recorded trees do not prove a no-op.".into(),
                    "Rewriting native descendants may stale their physical checkouts. No workspace is updated or repaired.".into(),
                    "Code restoration never rewinds agent memory. Checked downstream application bars all pre-existing affected sessions; no task is automatically rerun.".into(),
                    "This report alone is not permission to apply. Checked application compares a fresh report and requires explicit confirmation. External writers and filesystem changes are not locked.".into(),
                ],
            };
            anyhow::ensure!(serde_json::to_vec(&report)?.len() <= MAX_REPORT_BYTES,
                "restore impact exceeds the response limit; inspect native jj and task records directly");
            Ok::<_, anyhow::Error>(report)
        }).await.context("restore impact inspection timed out; no mutation was started")??)
    }

    async fn restore_preview_watermark(&self) -> anyhow::Result<i64> {
        let connection = self.database.connect()?;
        let mut rows = connection
            .query("SELECT COALESCE(MAX(id), 0) FROM restore_operations", ())
            .await?;
        Ok(rows
            .next()
            .await?
            .context("missing restore watermark")?
            .get(0)?)
    }

    async fn restore_impact_tasks(
        &self,
        node_id: i64,
        root: &std::path::Path,
        native: &[jj::ImpactRevision],
    ) -> anyhow::Result<Vec<RestoreImpactTask>> {
        let ids: BTreeSet<_> = native
            .iter()
            .map(|entry| &entry.revision.change_id)
            .collect();
        let connection = self.database.connect()?;
        // Seed by native ownership across workflows, then retain graph-only
        // descendants too. Native and prompt dependency graphs need not agree.
        let mut rows = connection
            .query(
                "WITH RECURSIVE affected(id) AS (
                SELECT ?1 UNION
                SELECT node_id FROM node_changes WHERE repository_path = ?2
                    AND change_id IN (SELECT value FROM json_each(?3)) UNION
                SELECT c.target_id FROM connections c JOIN affected a ON c.source_id = a.id
             )
             SELECT n.id, n.workflow_id, n.prompt, c.repository_path, c.change_id,
                    w.repository_path, w.change_id, w.name, w.directory, w.ready
             FROM nodes n LEFT JOIN node_changes c ON c.node_id = n.id
             LEFT JOIN node_workspaces w ON w.node_id = n.id
             WHERE n.id IN (SELECT id FROM affected)
               AND NOT EXISTS (SELECT 1 FROM deleted_nodes WHERE node_id = n.id)
             ORDER BY n.id",
                turso::params![
                    node_id,
                    root.to_str().context("non-UTF-8 repository")?,
                    serde_json::to_string(&ids)?
                ],
            )
            .await?;
        let mut tasks = Vec::new();
        while let Some(row) = rows.next().await? {
            let association = row
                .get::<Option<String>>(3)?
                .map(|repository| -> anyhow::Result<_> {
                    Ok(ChangeAssociation {
                        repository_path: repository.into(),
                        change_id: row.get(4)?,
                    })
                })
                .transpose()?;
            let allocation = row
                .get::<Option<String>>(5)?
                .map(|repository| -> anyhow::Result<_> {
                    Ok(RestoreImpactAllocation {
                        workspace: WorkspaceAssociation {
                            repository_path: repository.into(),
                            change_id: row.get(6)?,
                            name: row.get(7)?,
                            directory: row.get::<String>(8)?.into(),
                        },
                        ready: row.get::<i64>(9)? == 1,
                    })
                })
                .transpose()?;
            tasks.push(RestoreImpactTask {
                node: Node {
                    id: row.get(0)?,
                    workflow_id: row.get(1)?,
                    prompt: row.get(2)?,
                },
                native_affected: association
                    .as_ref()
                    .is_some_and(|a| a.repository_path == root && ids.contains(&a.change_id)),
                association,
                allocation,
            });
        }
        Ok(tasks)
    }

    async fn restore_impact_sessions(
        &self,
        node_id: i64,
        tasks: &[RestoreImpactTask],
    ) -> anyhow::Result<Vec<RestoreImpactSession>> {
        let connection = self.database.connect()?;
        let ids: Vec<_> = tasks.iter().map(|task| task.node.id).collect();
        let mut rows = connection
            .query(
                "SELECT id, node_id, json_extract(record, '$.session_id') FROM agent_runs
             WHERE node_id IN (SELECT value FROM json_each(?1)) ORDER BY id",
                [serde_json::to_string(&ids)?],
            )
            .await?;
        let mut sessions = BTreeMap::new();
        while let Some(row) = rows.next().await? {
            let owner = row.get::<i64>(1)?;
            let session_id = row.get::<Option<String>>(2)?;
            let entry = sessions
                .entry((owner, session_id.clone()))
                .or_insert_with(|| RestoreImpactSession {
                    node_id: owner,
                    session_id,
                    run_ids: Vec::new(),
                    source_node: owner == node_id,
                });
            entry.run_ids.push(row.get(0)?);
        }
        Ok(sessions.into_values().collect())
    }

    async fn restore_impact_reservations(&self) -> anyhow::Result<(Vec<i64>, Vec<String>)> {
        let connection = self.database.connect()?;
        let mut rows = connection
            .query("SELECT id FROM agent_runs WHERE active = 1 ORDER BY id", ())
            .await?;
        let mut active = Vec::new();
        while let Some(row) = rows.next().await? {
            active.push(row.get::<i64>(0)?);
        }
        let mut blockers = Vec::new();
        if !active.is_empty() {
            blockers.push(
                "stop or recover all active/interrupted agent runs before applying restore".into(),
            );
        }
        let mut rows = connection
            .query(
                "SELECT 'restore', id FROM restore_operations WHERE active = 1
             UNION ALL SELECT 'dependency', id FROM dependency_operations WHERE active = 1",
                (),
            )
            .await?;
        while let Some(row) = rows.next().await? {
            blockers.push(format!(
                "unresolved {} operation {}; inspect and recover explicitly",
                row.get::<String>(0)?,
                row.get::<i64>(1)?
            ));
        }
        Ok((active, blockers))
    }
}

async fn observe_checkout(workspace: &jj::ImpactWorkspace) -> RestoreImpactCheckout {
    let result = async {
        let directory = workspace
            .directory
            .as_ref()
            .context("native workspace directory is unavailable")?;
        anyhow::ensure!(
            fs::symlink_metadata(directory)?.file_type().is_dir(),
            "workspace directory is missing, not a directory, or a symlink"
        );
        anyhow::ensure!(
            jj::repository_root(directory).await? == *directory,
            "directory is no longer the recorded jj workspace root"
        );
        let checkout = jj::checked_out_revision(directory).await?;
        Ok::<_, anyhow::Error>(if checkout.commit_id == workspace.commit_id {
            RestoreImpactCheckout::CurrentRecorded
        } else {
            RestoreImpactCheckout::Stale {
                commit_id: checkout.commit_id,
            }
        })
    }
    .await;
    result.unwrap_or_else(|error| RestoreImpactCheckout::Unknown {
        detail: format!("{error:#}"),
    })
}
