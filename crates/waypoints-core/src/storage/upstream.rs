use std::time::Duration;

use anyhow::Context;
use tokio::time::timeout;

use super::{Storage, StorageError};
use crate::jj::{self, ChangeState};
use crate::run::{RunInput, UpstreamStatus};
use crate::workflow::{GraphConnectivity, NodeParent};

// Optional bookkeeping must not turn a slow/unavailable upstream into a run blocker.
const CAPTURE_TIMEOUT: Duration = Duration::from_millis(500);
// Leave room for the normal five-second RPC transport deadline.
const COMPARISON_TIMEOUT: Duration = Duration::from_secs(4);

impl Storage {
    pub(super) async fn capture_upstream_inputs(
        &self,
        workflow_id: i64,
        node_id: i64,
    ) -> Option<Vec<RunInput>> {
        timeout(
            CAPTURE_TIMEOUT,
            self.current_upstream_inputs(workflow_id, node_id),
        )
        .await
        .ok()
        .and_then(Result::ok)
    }

    /// Compares direct graph inputs without changing code, prompts, run eligibility, or stored history.
    ///
    /// This is not proof of fresh agent reasoning, native ancestry alignment, or a fresh checkout.
    /// Unavailable native state is returned as unknown, not unchanged.
    ///
    /// # Errors
    ///
    /// Returns an error when the saved run cannot be read.
    pub async fn run_upstream_status(&self, run_id: i64) -> Result<UpstreamStatus, StorageError> {
        let run = self.run(run_id).await?;
        let Some(saved) = run.upstream_inputs else {
            return Ok(UpstreamStatus::Unknown {
                detail: "upstream inputs were not recorded for this run".to_owned(),
            });
        };
        Ok(
            match timeout(
                COMPARISON_TIMEOUT,
                self.current_upstream_inputs(run.workflow_id, run.node_id),
            )
            .await
            {
                Ok(Ok(current)) if current == saved => UpstreamStatus::Unchanged,
                Ok(Ok(_)) => UpstreamStatus::Changed,
                Ok(Err(error)) => UpstreamStatus::Unknown {
                    detail: format!("{error:#}"),
                },
                Err(_) => UpstreamStatus::Unknown {
                    detail: "upstream comparison timed out".to_owned(),
                },
            },
        )
    }

    async fn current_upstream_inputs(
        &self,
        workflow_id: i64,
        node_id: i64,
    ) -> anyhow::Result<Vec<RunInput>> {
        let graph = self.workflow_graph(workflow_id).await?;
        anyhow::ensure!(
            graph.nodes.iter().any(|node| node.id == node_id),
            "node is no longer active in this workflow"
        );
        anyhow::ensure!(
            matches!(graph.connectivity, GraphConnectivity::Rooted),
            "upstream comparison requires a rooted workflow with native dependencies"
        );
        let base = graph.workflow.base.context("workflow has no saved base")?;
        anyhow::ensure!(
            jj::repository_root(&graph.workflow.project_path).await? == base.repository_path,
            "workflow's native repository root changed"
        );
        let mut inputs = Vec::new();
        if graph.base_nodes.contains(&node_id) {
            // A saved base is fixed, not the latest revision of its evolving change/bookmark.
            inputs.push(RunInput {
                parent: NodeParent::WorkflowBase {},
                commit_id: base.revision.commit_id,
            });
        }
        let mut upstream: Vec<_> = graph
            .connections
            .iter()
            .filter(|edge| edge.target_id == node_id)
            .map(|edge| edge.source_id)
            .collect();
        upstream.sort_unstable();
        for id in upstream {
            let change = self.refresh_node(workflow_id, id).await?;
            let ChangeState::Current { revision } = change.state else {
                anyhow::bail!("upstream node {id} has no unique current native revision");
            };
            inputs.push(RunInput {
                parent: NodeParent::Node { node_id: id },
                commit_id: revision.commit_id,
            });
        }
        anyhow::ensure!(!inputs.is_empty(), "node has no recorded dependencies");
        Ok(inputs)
    }
}
