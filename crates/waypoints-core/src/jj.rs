//! Native jj queries and explicit change/workspace creation; no application-owned history.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::time::timeout;

use crate::workflow::BaseSource;

/// A saved association with an evolving native change, not a frozen commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ChangeAssociation {
    /// The canonical workspace root discovered for the workflow's project.
    pub repository_path: PathBuf,
    /// The full native change ID, resolved when the association was saved.
    pub change_id: String,
}

/// One exact visible native revision of a jj change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Revision {
    /// The full evolving change identifier.
    pub change_id: String,
    /// The exact native commit identifier at query time.
    pub commit_id: String,
    /// The native commit description, preserved verbatim.
    pub description: String,
    /// Whether jj reports unresolved code conflicts.
    pub conflicted: bool,
}

/// Current repository state, fetched on demand rather than cached in the database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ChangeState {
    /// No change has been explicitly selected for this node.
    Unassociated,
    /// The saved change has no visible revision; its node and association remain.
    Missing,
    /// The change has one visible revision.
    Current {
        /// The current native revision.
        revision: Revision,
    },
    /// The change has multiple visible revisions; Waypoints does not choose one.
    Divergent {
        /// Two candidates proving divergence, ordered by commit ID (not all revisions).
        revisions: Vec<Revision>,
    },
}

// Bound each subprocess; creation also has a total deadline in storage.
const QUERY_TIMEOUT: Duration = Duration::from_secs(2);
// Materializing a real repository can take longer than a read-only query.
const WORKSPACE_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_OUTPUT: u64 = 1024 * 1024;
const REVISION_TEMPLATE: &str = r#""{\"change_id\":" ++ json(change_id) ++ ",\"commit_id\":" ++ json(commit_id) ++ ",\"description\":" ++ json(description) ++ ",\"conflicted\":" ++ json(conflict) ++ "}\n""#;

pub(crate) fn validate_change_id(value: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !value.is_empty()
            && value.len() <= 32
            && value.bytes().all(|byte| (b'k'..=b'z').contains(&byte)),
        "expected a jj change ID or unique change-ID prefix (letters k-z), not a commit ID or revset"
    );
    Ok(())
}

pub(crate) async fn repository_root(project: &Path) -> anyhow::Result<PathBuf> {
    let output = query(project, &["root"]).await?;
    let path = std::str::from_utf8(&output).context("jj workspace path must be UTF-8")?;
    Path::new(path.trim_end_matches(['\r', '\n']))
        .canonicalize()
        .context("resolving jj workspace root")
}

pub(crate) async fn revisions(repository: &Path, change_id: &str) -> anyhow::Result<Vec<Revision>> {
    validate_change_id(change_id)?;
    // change_id() reports divergence as multiple commits and an absent ID as an empty set.
    // User input is restricted to change-ID characters, never accepted as an arbitrary revset.
    let revset = format!("change_id(\"{change_id}\")");
    revision_set(repository, &revset).await
}

pub(crate) async fn resolve_base(repository: &Path, base: &str) -> anyhow::Result<Revision> {
    let revset = if base == "root()" {
        base.to_owned()
    } else if !base.is_empty()
        && base.len() <= 40
        && base.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        format!("commit_id(\"{base}\")")
    } else {
        validate_change_id(base)?;
        format!("change_id(\"{base}\")")
    };
    let mut revisions = revision_set(repository, &revset).await?;
    anyhow::ensure!(
        revisions.len() == 1,
        "select one existing, non-divergent base revision"
    );
    Ok(revisions.remove(0))
}

pub(crate) async fn resolve_workflow_base(
    repository: &Path,
    source: &BaseSource,
) -> anyhow::Result<Revision> {
    match source {
        BaseSource::Revision { id } => resolve_base(repository, id).await,
        BaseSource::Bookmark { name } => {
            anyhow::ensure!(
                !name.trim().is_empty() && !name.contains(['\0', '\r', '\n']),
                "base bookmark name must not be blank or contain control separators"
            );
            // Bookmark names are literal data, never revset source or glob patterns.
            let quoted = serde_json::to_string(name)?;
            let revset = format!("bookmarks(exact:{quoted})");
            let mut revisions = revision_set(repository, &revset).await?;
            anyhow::ensure!(
                revisions.len() == 1,
                "base bookmark {name:?} must have exactly one target; it is missing or conflicted"
            );
            Ok(revisions.remove(0))
        }
    }
}

pub(crate) async fn create_change(repository: &Path, base: &Revision) -> anyhow::Result<Revision> {
    // --no-edit reports the created commit using commit_summary. Override it with
    // marked JSON so identity comes from this command, never a racy heads() query.
    let template =
        format!("templates.commit_summary='\"waypoints-created:\" ++ {REVISION_TEMPLATE}'");
    let output = run(
        repository,
        &[
            "--config",
            &template,
            "--config",
            "ui.quiet=false",
            "new",
            "--no-edit",
            "-m",
            "",
            &base.commit_id,
        ],
    )
    .await?;
    let stderr = std::str::from_utf8(&output.stderr)?;
    let mut revisions = stderr
        .lines()
        .filter_map(|line| line.split_once("waypoints-created:").map(|(_, json)| json))
        .map(serde_json::from_str::<Revision>)
        .collect::<Result<Vec<_>, _>>()
        .context("decoding created jj change; inspect jj before retrying")?;
    anyhow::ensure!(
        revisions.len() == 1,
        "jj did not report exactly one created change; inspect jj before retrying"
    );
    Ok(revisions.remove(0))
}

async fn revision_set(repository: &Path, revset: &str) -> anyhow::Result<Vec<Revision>> {
    let args = [
        "log",
        "--no-graph",
        "--limit",
        "2",
        "-r",
        revset,
        "-T",
        REVISION_TEMPLATE,
    ];
    let output = query(repository, &args).await?;
    let mut revisions: Vec<Revision> = serde_json::Deserializer::from_slice(&output)
        .into_iter()
        .collect::<Result<_, _>>()
        .context(
            "decoding jj revisions (requires jj with JSON templates and change_id() revsets)",
        )?;
    revisions.sort_by(|left, right| left.commit_id.cmp(&right.commit_id));
    Ok(revisions)
}

async fn query(repository: &Path, args: &[&str]) -> anyhow::Result<Vec<u8>> {
    Ok(run(repository, args).await?.stdout)
}

#[derive(Debug, Deserialize)]
pub(crate) struct NativeWorkspace {
    pub name: String,
    pub directory: Option<PathBuf>,
    pub change_id: String,
}

pub(crate) async fn workspaces(repository: &Path) -> anyhow::Result<Vec<NativeWorkspace>> {
    let template = r#""{\"name\":" ++ json(name) ++ ",\"directory\":" ++ json(root) ++ ",\"change_id\":" ++ json(target.change_id()) ++ "}\n""#;
    let output = query(repository, &["workspace", "list", "-T", template]).await?;
    serde_json::Deserializer::from_slice(&output)
        .into_iter()
        .collect::<Result<_, _>>()
        .context("decoding jj workspaces (requires workspace root templates)")
}

pub(crate) async fn current_revision(repository: &Path) -> anyhow::Result<Revision> {
    let mut revisions = revision_set(repository, "@").await?;
    anyhow::ensure!(revisions.len() == 1, "expected one working-copy revision");
    Ok(revisions.remove(0))
}

pub(crate) async fn snapshot_workspace(
    workspace: &crate::workspace::WorkspaceAssociation,
    expected: &Revision,
) -> anyhow::Result<Revision> {
    anyhow::ensure!(
        repository_root(&workspace.directory).await? == workspace.directory,
        "snapshot directory no longer resolves to its saved workspace root"
    );
    let current = current_revision(&workspace.directory).await?;
    let checked_out = checked_out_revision(&workspace.directory).await?;
    anyhow::ensure!(
        current == *expected && checked_out == *expected,
        "workspace changed or checkout became stale before snapshotting"
    );
    ensure_mutable(&workspace.directory, expected).await?;
    // Query @ in the same command that snapshots, rather than querying a moving
    // operation head afterwards. @ refuses divergent operations; stale checkouts
    // must fail instead of updating files to a different revision.
    let output = run_command(
        &workspace.directory,
        &[
            "--at-op=@",
            "--config",
            "snapshot.auto-update-stale=false",
            "--quiet",
            "log",
            "--no-graph",
            "-r",
            "@",
            "-T",
            REVISION_TEMPLATE,
        ],
        WorkingCopy::Update,
        WORKSPACE_TIMEOUT,
    )
    .await?;
    anyhow::ensure!(
        output.stderr.is_empty(),
        "jj snapshot reported diagnostics; some files may not have been recorded; inspect native jj state: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    let revision: Revision = serde_json::from_slice(&output.stdout)
        .context("decoding snapshotted revision; native snapshot may have succeeded")?;
    anyhow::ensure!(
        revision.change_id == workspace.change_id,
        "snapshot did not retain the run's change; inspect native jj state"
    );
    Ok(revision)
}

pub(crate) async fn checked_out_revision(repository: &Path) -> anyhow::Result<Revision> {
    // Registry @ can already point at the intended change before its files are
    // materialized. The working copy's own operation records the completed checkout.
    // Do not run status/update-stale: those would snapshot or alter local files.
    let output = query(repository, &["debug", "working-copy"]).await?;
    let operation = checkout_operation(std::str::from_utf8(&output)?)?;
    let revset = format!("at_operation({operation}, @)");
    let mut revisions = revision_set(repository, &revset).await.context(
        "cannot resolve the recorded checkout; inspect this workspace with native jj before manual recovery",
    )?;
    anyhow::ensure!(
        revisions.len() == 1,
        "cannot identify one checked-out revision"
    );
    Ok(revisions.remove(0))
}

fn checkout_operation(output: &str) -> anyhow::Result<&str> {
    anyhow::ensure!(
        output
            .lines()
            .filter(|line| *line == "Type: \"local\"")
            .count()
            == 1,
        "cannot verify checkout: unsupported jj working-copy type or debug output"
    );
    let mut operations = output.lines().filter_map(|line| {
        line.strip_prefix("Current operation: OperationId(\"")?
            .strip_suffix("\")")
    });
    let operation = operations
        .next()
        .context("cannot verify checkout: jj debug working-copy did not report its operation")?;
    anyhow::ensure!(
        operations.next().is_none()
            && operation.len() == 128
            && operation.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "cannot verify checkout: unsupported jj working-copy operation format"
    );
    Ok(operation)
}

pub(crate) async fn ensure_mutable(repository: &Path, revision: &Revision) -> anyhow::Result<()> {
    let output = query(
        repository,
        &[
            "log",
            "--no-graph",
            "-r",
            &revision.commit_id,
            "-T",
            "immutable",
        ],
    )
    .await?;
    anyhow::ensure!(
        output == b"false",
        "cannot edit an immutable revision in a node workspace"
    );
    Ok(())
}

pub(crate) async fn create_workspace(
    association: &crate::workspace::WorkspaceAssociation,
    revision: &Revision,
) -> anyhow::Result<()> {
    let directory = association
        .directory
        .to_str()
        .context("workspace path must be UTF-8")?;
    run_command(
        &association.repository_path,
        &[
            "workspace",
            "add",
            "--name",
            &association.name,
            "--no-colocate",
            "--sparse-patterns",
            "full",
            "-r",
            &revision.commit_id,
            "--",
            directory,
        ],
        WorkingCopy::Ignore,
        WORKSPACE_TIMEOUT,
    )
    .await?;
    // add -r makes a temporary empty child. edit selects the node's actual change
    // and lets jj discard that empty child. Neither command snapshots the source.
    run_command(
        &association.directory,
        &["edit", &revision.commit_id],
        WorkingCopy::Ignore,
        WORKSPACE_TIMEOUT,
    )
    .await?;
    // Only the new directory is updated. With --ignore-working-copy, add/edit
    // deliberately left its filesystem at the initial empty checkout.
    run_command(
        &association.directory,
        &["workspace", "update-stale"],
        WorkingCopy::Update,
        WORKSPACE_TIMEOUT,
    )
    .await?;
    Ok(())
}

struct CommandOutput {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

async fn run(repository: &Path, args: &[&str]) -> anyhow::Result<CommandOutput> {
    run_command(repository, args, WorkingCopy::Ignore, QUERY_TIMEOUT).await
}

enum WorkingCopy {
    Ignore,
    Update,
}

async fn run_command(
    repository: &Path,
    args: &[&str],
    working_copy: WorkingCopy,
    deadline: Duration,
) -> anyhow::Result<CommandOutput> {
    let mut command = Command::new("jj");
    if matches!(working_copy, WorkingCopy::Ignore) {
        command.args(["--ignore-working-copy", "--at-op=@"]);
    }
    command
        .current_dir(repository)
        .args(["--color=never", "--no-pager"])
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    // @ pins the current operation head and refuses concurrent operation heads instead
    // of merging them. --ignore-working-copy prevents automatic snapshots and Git import.
    timeout(deadline, async {
        let mut child = command
            .spawn()
            .context("running jj; install jj and ensure it is on the backend's PATH")?;
        let stdout = child.stdout.take().context("opening jj stdout")?;
        let stderr = child.stderr.take().context("opening jj stderr")?;
        let (stdout, stderr, status) =
            tokio::try_join!(read_output(stdout), read_output(stderr), child.wait(),)
                .context("reading jj output")?;
        anyhow::ensure!(
            status.success(),
            "jj command failed: {}",
            String::from_utf8_lossy(&stderr).trim()
        );
        Ok(CommandOutput { stdout, stderr })
    })
    .await
    .context("jj command timed out")?
}

async fn read_output(stream: impl AsyncRead + Unpin) -> std::io::Result<Vec<u8>> {
    let mut output = Vec::new();
    stream.take(MAX_OUTPUT + 1).read_to_end(&mut output).await?;
    if output.len() as u64 > MAX_OUTPUT {
        return Err(std::io::Error::other("jj output exceeds 1 MiB"));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::{MAX_OUTPUT, checkout_operation, read_output, validate_change_id};

    #[test]
    fn checkout_operation_parser_fails_closed() -> anyhow::Result<()> {
        let operation = "a".repeat(128);
        let valid = format!(
            "Type: \"local\"\nCurrent operation: OperationId(\"{operation}\")\nCurrent tree: ignored\n"
        );
        assert_eq!(checkout_operation(&valid)?, operation);
        for invalid in [
            String::new(),
            valid.replace("\"local\"", "\"other\""),
            valid.replace("Current operation:", "Changed format:"),
            valid.replace(&operation, "abc"),
            valid.replace(&operation, &"z".repeat(128)),
            format!("{valid}Current operation: OperationId(\"{operation}\")\n"),
        ] {
            assert!(checkout_operation(&invalid).is_err(), "{invalid}");
        }
        Ok(())
    }

    #[test]
    fn query_output_is_bounded() -> anyhow::Result<()> {
        tokio::runtime::Builder::new_current_thread()
            .build()?
            .block_on(async {
                assert_eq!(read_output(&b"small result"[..]).await?, b"small result");
                let bytes = vec![b'x'; MAX_OUTPUT as usize + 1];
                assert!(read_output(&bytes[..]).await.is_err());
                Ok(())
            })
    }

    #[test]
    fn only_change_ids_are_accepted() {
        for valid in [
            "k",
            "wrtnmyrzzzuyumnlozyloulmxmlwuyxp",
            "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
        ] {
            assert!(validate_change_id(valid).is_ok(), "{valid}");
        }
        for invalid in [
            "",
            "@",
            "all()",
            "abc123",
            "--help",
            "k | root()",
            "K",
            " k",
            "k\n",
            "kkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkk",
        ] {
            assert!(validate_change_id(invalid).is_err(), "{invalid:?}");
        }
    }
}
