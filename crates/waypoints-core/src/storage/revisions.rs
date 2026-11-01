//! Read-only native revision browsing with stateless, operation-pinned pagination.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use super::{Storage, StorageError};
use crate::jj::{self, RevisionEvolution, RevisionReview};

const HISTORY_TIMEOUT: Duration = Duration::from_secs(4);
const MAX_CURSOR_BYTES: usize = 64 * 1024;

/// One native history entry annotated with this node's saved agent-run references.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct NodeRevisionEntry {
    /// Native history data, including exact revision and predecessor links.
    pub evolution: RevisionEvolution,
    /// Whether this commit is a visible head at the page's pinned native operation.
    pub is_current: bool,
    /// IDs of this node's runs whose saved snapshot exactly matches this commit.
    pub run_ids: Vec<i64>,
}

/// Bounded native history, not an application-owned snapshot collection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct NodeRevisionPage {
    /// Owning workflow.
    pub workflow_id: i64,
    /// Owning node.
    pub node_id: i64,
    /// Associated change; squash history may also contain related changes.
    pub change_id: String,
    /// Repository view pinned across every page, not a claim about live state.
    pub native_operation: String,
    /// All visible heads in that view; multiple heads mean divergence.
    pub current_commit_ids: Vec<String>,
    /// Topologically ordered evolution entries, newer before their predecessors.
    pub entries: Vec<NodeRevisionEntry>,
    /// Opaque continuation; none means native traversal reached its end.
    pub next_cursor: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    version: u8,
    workflow_id: i64,
    node_id: i64,
    repository: PathBuf,
    change_id: String,
    operation: String,
    heads: Vec<String>,
    frontier: Vec<String>,
}

impl Storage {
    /// Lists native revisions without snapshotting, switching, or requiring an editing workspace.
    ///
    /// Page size is 1–100. Cursors pin the native operation, association and
    /// traversal frontier; refresh without a cursor to observe later rewrites.
    /// Saved-run annotations are live metadata, not a frozen conversation view.
    ///
    /// # Errors
    ///
    /// Refuses invalid pages/cursors, unassociated or missing visible changes,
    /// moved/unavailable repositories, collected history, oversized output and
    /// native/storage failures. Never substitutes a partial list for success.
    pub async fn node_revisions(
        &self,
        workflow_id: i64,
        node_id: i64,
        page_size: usize,
        cursor: Option<&str>,
    ) -> Result<NodeRevisionPage, StorageError> {
        Ok(tokio::time::timeout(HISTORY_TIMEOUT, async {
            anyhow::ensure!((1..=100).contains(&page_size), "page size must be between 1 and 100");
            let association = self.revision_repository(workflow_id, node_id).await?;
            let mut cursor = if let Some(encoded) = cursor {
                anyhow::ensure!(encoded.len() <= MAX_CURSOR_BYTES, "revision cursor is too large");
                let cursor: Cursor = serde_json::from_str(encoded).context("invalid revision cursor")?;
                anyhow::ensure!(
                    cursor.version == 1 && cursor.workflow_id == workflow_id && cursor.node_id == node_id
                        && cursor.repository == association.repository_path && cursor.change_id == association.change_id,
                    "revision cursor belongs to another node/association or unsupported version; refresh history"
                );
                anyhow::ensure!(
                    cursor.operation.len() == 128 && cursor.operation.bytes().all(|b| b.is_ascii_hexdigit())
                        && !cursor.heads.is_empty() && !cursor.frontier.is_empty(),
                    "invalid revision cursor operation/heads/frontier"
                );
                for id in cursor.heads.iter().chain(&cursor.frontier) { jj::validate_commit_id(id)?; }
                let heads = jj::evolution_heads(&cursor.repository, &cursor.change_id, &cursor.operation)
                    .await?.into_iter().map(|revision| revision.commit_id).collect::<Vec<_>>();
                anyhow::ensure!(heads == cursor.heads,
                    "revision cursor heads do not match its pinned native operation");
                cursor
            } else {
                let operation = jj::operation_id(&association.repository_path).await?;
                let heads = jj::evolution_heads(&association.repository_path, &association.change_id, &operation)
                    .await?.into_iter().map(|revision| revision.commit_id).collect::<Vec<_>>();
                anyhow::ensure!(!heads.is_empty(),
                    "change has no visible revision; saved-run reviews may still be available");
                Cursor {
                    version: 1, workflow_id, node_id, repository: association.repository_path.clone(),
                    change_id: association.change_id.clone(), operation, frontier: heads.clone(), heads,
                }
            };
            let native = jj::evolution_page(
                &cursor.repository, &cursor.operation, &cursor.frontier, page_size,
            ).await?;
            anyhow::ensure!(!native.is_empty(), "native evolution became unavailable; refresh history");
            let mut run_ids = self.revision_runs(node_id, &native).await?;
            let shown: BTreeSet<_> = native.iter().map(|entry| entry.revision.commit_id.clone()).collect();
            let mut frontier: BTreeSet<_> = cursor.frontier.iter().cloned().collect();
            for entry in &native {
                frontier.extend(entry.predecessor_commit_ids.iter().cloned());
            }
            // Native topological ordering guarantees no emitted entry is reachable
            // from a remaining frontier. This avoids an ever-growing "seen" cursor
            // and does not rescan/re-emit earlier pages.
            frontier.retain(|id| !shown.contains(id));
            cursor.frontier = frontier.into_iter().collect();
            let entries = native.into_iter().map(|evolution| NodeRevisionEntry {
                is_current: cursor.heads.contains(&evolution.revision.commit_id),
                run_ids: run_ids.remove(&evolution.revision.commit_id).unwrap_or_default(),
                evolution,
            }).collect();
            let next_cursor = if cursor.frontier.is_empty() { None } else {
                let encoded = serde_json::to_string(&cursor)?;
                anyhow::ensure!(encoded.len() <= MAX_CURSOR_BYTES,
                    "native evolution frontier exceeds cursor limit; inspect history with native jj");
                Some(encoded)
            };
            let page = NodeRevisionPage {
                workflow_id, node_id, change_id: association.change_id, native_operation: cursor.operation,
                current_commit_ids: cursor.heads, entries, next_cursor,
            };
            anyhow::ensure!(serde_json::to_vec(&page)?.len() <= 900 * 1024,
                "revision page exceeds response limit; request a smaller page or inspect native jj");
            Ok::<_, anyhow::Error>(page)
        }).await.context("revision history timed out; no files or native state changed")??)
    }

    /// Previews an exact commit in the node's associated repository without editing code.
    ///
    /// Related-change commits from native squash evolution may also be inspected.
    /// The selector must be a full lowercase commit ID, never a revset or prefix.
    ///
    /// # Errors
    ///
    /// Refuses invalid IDs, invalid membership/association, unavailable native
    /// objects, oversized review output and native/storage failures.
    pub async fn review_node_revision(
        &self,
        workflow_id: i64,
        node_id: i64,
        commit_id: &str,
    ) -> Result<RevisionReview, StorageError> {
        Ok(tokio::time::timeout(HISTORY_TIMEOUT, async {
            jj::validate_commit_id(commit_id)?;
            let association = self.revision_repository(workflow_id, node_id).await?;
            let revision = jj::resolve_base(&association.repository_path, commit_id).await?;
            jj::review(&association.repository_path, revision).await
        })
        .await
        .context("exact revision review timed out")??)
    }

    async fn revision_repository(
        &self,
        workflow_id: i64,
        node_id: i64,
    ) -> anyhow::Result<jj::ChangeAssociation> {
        self.node(workflow_id, node_id).await?;
        let association = self
            .node_change_association(node_id)
            .await?
            .context("node has no native change association")?;
        let project = self.project_for_node(workflow_id, node_id).await?;
        anyhow::ensure!(
            jj::repository_root(&project).await? == association.repository_path,
            "project's jj workspace root changed; explicitly associate the node again"
        );
        Ok(association)
    }

    async fn revision_runs(
        &self,
        node_id: i64,
        entries: &[RevisionEvolution],
    ) -> anyhow::Result<HashMap<String, Vec<i64>>> {
        // Read only IDs for the requested page, not every prompt/session record.
        let placeholders = (2..entries.len() + 2)
            .map(|n| format!("?{n}"))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT id, json_extract(record, '$.snapshot.revision.commit_id') FROM agent_runs
             WHERE node_id = ?1 AND json_extract(record, '$.snapshot.status') = 'saved'
               AND json_extract(record, '$.snapshot.revision.commit_id') IN ({placeholders}) ORDER BY id"
        );
        let mut params = vec![turso::Value::Integer(node_id)];
        params.extend(
            entries
                .iter()
                .map(|entry| turso::Value::Text(entry.revision.commit_id.clone())),
        );
        let connection = self.database.connect()?;
        let mut rows = connection.query(&sql, params).await?;
        let mut result = HashMap::<String, Vec<i64>>::new();
        while let Some(row) = rows.next().await? {
            result.entry(row.get(1)?).or_default().push(row.get(0)?);
        }
        Ok(result)
    }
}
