//! Read-only, operation-pinned facts for restore impact; never a speculative mutation.

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Deserialize;

use super::Revision;

#[derive(Debug, Deserialize)]
pub(crate) struct ImpactRevision {
    pub revision: Revision,
    pub parent_commit_ids: Vec<String>,
    pub immutable: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ImpactWorkspace {
    pub name: String,
    pub directory: Option<PathBuf>,
    pub commit_id: String,
}

pub(crate) async fn impact_revisions(
    repository: &Path,
    operation: &str,
    revisions: &str,
) -> anyhow::Result<Vec<ImpactRevision>> {
    let template = r#"json({"revision" => {"change_id" => change_id, "commit_id" => commit_id, "description" => description, "conflicted" => conflict}, "parent_commit_ids" => parents.map(|p| p.commit_id()), "immutable" => immutable}) ++ "\n""#;
    let output = super::run(
        repository,
        &[
            &format!("--at-op={operation}"),
            "log",
            "--no-graph",
            "-r",
            revisions,
            "-T",
            template,
        ],
    )
    .await?;
    anyhow::ensure!(
        output.stderr.is_empty(),
        "jj reported restore-impact diagnostics: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    serde_json::Deserializer::from_slice(&output.stdout)
        .into_iter()
        .collect::<Result<_, _>>()
        .context("decoding restore-impact revisions")
}

pub(crate) async fn impact_workspaces(
    repository: &Path,
    operation: &str,
) -> anyhow::Result<Vec<ImpactWorkspace>> {
    let template =
        r#"json({"name" => name, "directory" => root, "commit_id" => target.commit_id()}) ++ "\n""#;
    let output = super::run(
        repository,
        &[
            &format!("--at-op={operation}"),
            "workspace",
            "list",
            "-T",
            template,
        ],
    )
    .await?;
    anyhow::ensure!(
        output.stderr.is_empty(),
        "jj reported workspace-impact diagnostics"
    );
    serde_json::Deserializer::from_slice(&output.stdout)
        .into_iter()
        .collect::<Result<_, _>>()
        .context("decoding restore-impact workspaces")
}

pub(crate) async fn impact_tree_changed(
    repository: &Path,
    operation: &str,
    current: &Revision,
    selected: &Revision,
) -> anyhow::Result<bool> {
    let output = super::run(
        repository,
        &[
            &format!("--at-op={operation}"),
            "diff",
            "--from",
            &current.commit_id,
            "--to",
            &selected.commit_id,
            "--summary",
        ],
    )
    .await?;
    anyhow::ensure!(
        output.stderr.is_empty(),
        "jj reported tree comparison diagnostics"
    );
    Ok(!output.stdout.is_empty())
}
