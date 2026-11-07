use anyhow::Context as _;
use serde::{Deserialize, Serialize};

use super::{Storage, StorageError};
use crate::run::{
    ContinuationAssessment, ContinuationBlocker, NodeRun, Run, RunPage, RunState, WorkflowRunPage,
};
use crate::workspace::WorkspaceState;

// Leave room for transport envelopes and live permission overlays.
const PAGE_BYTES: usize = 512 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunCursor {
    workflow_id: i64,
    node_id: i64,
    through: i64,
    before: i64,
}

impl Storage {
    /// Returns the newest saved run without loading the node's full history.
    ///
    /// # Errors
    ///
    /// Refuses invalid workflow membership or unreadable saved records.
    pub async fn latest_node_run(
        &self,
        workflow_id: i64,
        node_id: i64,
    ) -> Result<Option<Run>, StorageError> {
        Ok(self.node_run_page(workflow_id, node_id, 1, None).await?.runs.into_iter().next())
    }

    /// Reads a bounded, newest-first history pinned against newly inserted runs.
    ///
    /// Cursors are opaque and scoped to one workflow/node. Saved lifecycle state
    /// may advance between pages; the insertion boundary does not freeze outcomes.
    ///
    /// # Errors
    ///
    /// Refuses invalid membership, page sizes outside 1–100, invalid/cross-node
    /// cursors, oversized records, and database/decoding failures.
    pub async fn node_run_page(
        &self,
        workflow_id: i64,
        node_id: i64,
        page_size: usize,
        cursor: Option<&str>,
    ) -> Result<RunPage, StorageError> {
        Ok(async {
            anyhow::ensure!((1..=100).contains(&page_size), "run page size must be 1–100");
            let connection = self.database.connect()?;
            let mut membership = connection.query(
                "SELECT id FROM nodes WHERE workflow_id = ?1 AND id = ?2", [workflow_id, node_id],
            ).await?;
            anyhow::ensure!(membership.next().await?.is_some(), "node not found in workflow");
            let cursor = match cursor {
                Some(cursor) => {
                    anyhow::ensure!(cursor.len() <= 1024, "run cursor is too large");
                    let cursor: RunCursor = serde_json::from_str(cursor)?;
                    anyhow::ensure!(
                        cursor.workflow_id == workflow_id && cursor.node_id == node_id
                            && cursor.before > 0 && cursor.through >= cursor.before,
                        "run cursor does not match the requested history"
                    );
                    cursor
                }
                None => {
                    let mut rows = connection.query(
                        "SELECT COALESCE(MAX(id), 0) FROM agent_runs WHERE node_id = ?1", [node_id],
                    ).await?;
                    let through = rows.next().await?.context("missing run maximum")?.get(0)?;
                    RunCursor { workflow_id, node_id, through, before: i64::MAX }
                }
            };
            let mut rows = connection.query(
                "SELECT id, record FROM agent_runs
                 WHERE node_id = ?1 AND id <= ?2 AND id < ?3 ORDER BY id DESC LIMIT ?4",
                turso::params![node_id, cursor.through, cursor.before, page_size as i64 + 1],
            ).await?;
            let mut runs: Vec<Run> = Vec::new();
            let mut bytes = 0;
            let mut more = false;
            while let Some(row) = rows.next().await? {
                let record: String = row.get(1)?;
                if runs.len() == page_size || bytes + record.len() > PAGE_BYTES {
                    anyhow::ensure!(!runs.is_empty(), "saved run exceeds page byte limit");
                    more = true;
                    break;
                }
                bytes += record.len();
                let mut run: Run = serde_json::from_str(&record)?;
                run.id = row.get(0)?;
                runs.push(run);
            }
            let next_cursor = if more {
                Some(serde_json::to_string(&RunCursor {
                    before: runs.last().context("nonempty run page")?.id,
                    ..cursor
                })?)
            } else { None };
            Ok::<_, anyhow::Error>(RunPage { runs, next_cursor })
        }.await?)
    }

    /// Lists latest runs across visible nodes without fetching every run per node.
    ///
    /// This is a paginated observation, not a frozen workflow snapshot.
    ///
    /// # Errors
    ///
    /// Refuses missing workflows, page sizes outside 1–100, invalid node cursors,
    /// oversized records, or database/decoding failures.
    pub async fn workflow_run_page(
        &self,
        workflow_id: i64,
        page_size: usize,
        after_node: Option<i64>,
    ) -> Result<WorkflowRunPage, StorageError> {
        Ok(async {
            self.workflow(workflow_id).await?;
            anyhow::ensure!((1..=100).contains(&page_size), "overview page size must be 1–100");
            anyhow::ensure!(after_node.is_none_or(|id| id > 0), "invalid overview node cursor");
            let connection = self.database.connect()?;
            let mut rows = connection.query(
                "SELECT n.id, r.id, r.record FROM nodes n
                 LEFT JOIN agent_runs r ON r.id = (SELECT MAX(id) FROM agent_runs WHERE node_id = n.id)
                 WHERE n.workflow_id = ?1 AND n.id > ?2
                 AND NOT EXISTS (SELECT 1 FROM deleted_nodes WHERE node_id = n.id)
                 ORDER BY n.id LIMIT ?3",
                turso::params![workflow_id, after_node.unwrap_or(0), page_size as i64 + 1],
            ).await?;
            let mut nodes: Vec<NodeRun> = Vec::new();
            let mut bytes = 0;
            let mut more = false;
            while let Some(row) = rows.next().await? {
                let record: Option<String> = row.get(2)?;
                let size = record.as_ref().map_or(0, String::len);
                if nodes.len() == page_size || bytes + size > PAGE_BYTES {
                    anyhow::ensure!(!nodes.is_empty(), "saved run exceeds overview byte limit");
                    more = true;
                    break;
                }
                bytes += size;
                let latest = record.map(|record| -> anyhow::Result<Run> {
                    let mut run: Run = serde_json::from_str(&record)?;
                    run.id = row.get(1)?;
                    Ok(run)
                }).transpose()?;
                nodes.push(NodeRun { node_id: row.get(0)?, latest });
            }
            let next_node = more.then(|| nodes.last().expect("nonempty overview page").node_id);
            Ok::<_, anyhow::Error>(WorkflowRunPage { nodes, next_node })
        }.await?)
    }

    /// Assesses local continuation prerequisites without mutating files or readiness.
    ///
    /// Adapter loading capability is checked only when executing the continuation.
    ///
    /// # Errors
    ///
    /// Returns an error for missing records, unreadable native state, or database failure.
    pub async fn assess_continuation(&self, source_run_id: i64) -> Result<ContinuationAssessment, StorageError> {
        let source = self.run(source_run_id).await?;
        let blocked = |reason, detail: String| ContinuationAssessment::Blocked { reason, detail };
        if !matches!(source.state, RunState::Completed | RunState::Cancelled | RunState::Failed { .. } | RunState::Recovered { .. }) {
            return Ok(blocked(ContinuationBlocker::RunNotStopped, "Continue requires confirmed cleanup or explicit recovery.".into()));
        }
        let Some(session) = source.session_id.as_deref().filter(|id| !id.is_empty()) else {
            return Ok(blocked(ContinuationBlocker::MissingSession, "No saved ACP session. Start a fresh run.".into()));
        };
        if self.session_was_restored(&source).await.map_err(StorageError::from)? {
            return Ok(blocked(ContinuationBlocker::RestoredSession, "Code restoration invalidated this session for continuation; explicitly start a fresh run.".into()));
        }
        let connection = self.database.connect().map_err(anyhow::Error::from)?;
        let mut rows = connection.query(
            "SELECT MAX(id) FROM agent_runs WHERE node_id = ?1 AND json_extract(record, '$.session_id') = ?2",
            turso::params![source.node_id, session],
        ).await.map_err(anyhow::Error::from)?;
        let latest: i64 = rows.next().await.map_err(anyhow::Error::from)?
            .context("missing session maximum")?.get(0).map_err(anyhow::Error::from)?;
        if latest != source.id {
            return Ok(blocked(ContinuationBlocker::Superseded, format!("Session has a newer run {latest}. Continue that run instead.")));
        }
        if let Err(error) = self.ensure_node_idle(source.node_id).await {
            if error.failure().is_some() {
                return Ok(blocked(ContinuationBlocker::NodeBusy, error.to_string()));
            }
            return Err(error);
        }
        let workspace = self.node_workspace(source.workflow_id, source.node_id).await?;
        if workspace.workspace.as_ref().is_none_or(|allocation| allocation.directory != source.directory || allocation.change_id != source.change_id) {
            return Ok(blocked(ContinuationBlocker::WorkspaceChanged, "Workspace or change differs from the saved session.".into()));
        }
        if !matches!(workspace.state, WorkspaceState::Ready { .. }) {
            return Ok(blocked(ContinuationBlocker::WorkspaceNotReady, "Workspace is not ready. Inspect and explicitly reconcile it before continuing.".into()));
        }
        if source.executable.canonicalize().ok().as_ref() != Some(&source.executable) || !source.executable.is_file() {
            return Ok(blocked(ContinuationBlocker::ExecutableChanged, "Saved ACP executable is missing or resolves to a different path.".into()));
        }
        Ok(ContinuationAssessment::Eligible { run_id: source.id })
    }
}
