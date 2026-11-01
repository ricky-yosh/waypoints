//! Bounded native evolution queries pinned to one repository operation.

use super::Revision;
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// One native evolution entry; predecessor links are rewrites, not code parents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RevisionEvolution {
    /// Exact code revision, possibly from a related change after squash.
    pub revision: Revision,
    /// Native predecessor commits, preserving branching and converging history.
    pub predecessor_commit_ids: Vec<String>,
    /// Native operation which recorded this version.
    pub operation_id: String,
    /// Native operation description, not an invented revision title.
    pub operation_description: String,
    /// Native operation end time in RFC 3339 form.
    pub timestamp: String,
}

const EVOLUTION_TEMPLATE: &str = r#"json({"revision" => {"change_id" => commit.change_id(), "commit_id" => commit.commit_id(), "description" => commit.description(), "conflicted" => commit.conflict()}, "predecessor_commit_ids" => predecessors.map(|p| p.commit_id()), "operation_id" => operation.id(), "operation_description" => operation.description(), "timestamp" => operation.time().end()}) ++ "\n""#;

pub(crate) fn validate_commit_id(id: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        id.len() == 40
            && id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()),
        "select an exact full lowercase jj commit ID, not a prefix, change ID or revset"
    );
    Ok(())
}

pub(crate) async fn evolution_heads(
    repository: &Path,
    change_id: &str,
    operation: &str,
) -> anyhow::Result<Vec<Revision>> {
    super::validate_change_id(change_id)?;
    let output = super::run(
        repository,
        &[
            &format!("--at-op={operation}"),
            "log",
            "--no-graph",
            "-r",
            &format!("change_id(\"{change_id}\")"),
            "-T",
            super::REVISION_TEMPLATE,
        ],
    )
    .await?;
    anyhow::ensure!(
        output.stderr.is_empty(),
        "jj reported diagnostics while reading evolution heads"
    );
    serde_json::Deserializer::from_slice(&output.stdout)
        .into_iter()
        .collect::<Result<_, _>>()
        .context("reading visible evolution heads")
}

pub(crate) async fn evolution_page(
    repository: &Path,
    operation: &str,
    frontier: &[String],
    limit: usize,
) -> anyhow::Result<Vec<RevisionEvolution>> {
    anyhow::ensure!(!frontier.is_empty(), "evolution frontier must not be empty");
    for id in frontier {
        validate_commit_id(id)?;
    }
    let revisions = frontier
        .iter()
        .map(|id| format!("commit_id(\"{id}\")"))
        .collect::<Vec<_>>()
        .join(" | ");
    let output = super::run(
        repository,
        &[
            &format!("--at-op={operation}"),
            "evolog",
            "--no-graph",
            "--limit",
            &limit.to_string(),
            "-r",
            &revisions,
            "-T",
            EVOLUTION_TEMPLATE,
        ],
    )
    .await?;
    anyhow::ensure!(
        output.stderr.is_empty(),
        "jj reported evolution diagnostics; history may be unavailable: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    serde_json::Deserializer::from_slice(&output.stdout)
        .into_iter()
        .collect::<Result<_, _>>()
        .context("reading native evolution; history may be unavailable or collected")
}
