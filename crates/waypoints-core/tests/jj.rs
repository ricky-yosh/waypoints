use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;
use waypoints_core::jj::ChangeState;
use waypoints_core::storage::Storage;
use waypoints_core::workflow::BaseSource;
use waypoints_core::workspace::WorkspaceState;

struct Repository {
    directory: TempDir,
    project: PathBuf,
}

impl Repository {
    fn new() -> anyhow::Result<Self> {
        let directory = tempfile::tempdir()?;
        let project = directory.path().join("project");
        let repo = Self { directory, project };
        repo.jj_at(repo.directory.path(), &["git", "init", "project"])?;
        Ok(repo)
    }

    fn jj_at(&self, path: &Path, args: &[&str]) -> anyhow::Result<String> {
        let output = Command::new("jj")
            .current_dir(path)
            .env("JJ_CONFIG", "")
            .args([
                "--config",
                "user.name='Waypoints test'",
                "--config",
                "user.email='test@example.invalid'",
                "--no-pager",
                "--color=never",
            ])
            .args(args)
            .output()?;
        anyhow::ensure!(
            output.status.success(),
            "jj {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(String::from_utf8(output.stdout)?)
    }

    fn jj(&self, args: &[&str]) -> anyhow::Result<String> {
        self.jj_at(&self.project, args)
    }

    fn value(&self, template: &str) -> anyhow::Result<String> {
        self.jj(&["log", "--no-graph", "-r", "@", "-T", template])
    }

    fn operation(&self) -> anyhow::Result<String> {
        self.jj(&[
            "--ignore-working-copy",
            "--at-op=@",
            "op",
            "log",
            "--no-graph",
            "--limit",
            "1",
            "-T",
            "id",
        ])
    }
}

fn run(test: impl Future<Output = anyhow::Result<()>>) -> anyhow::Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(test)
}

fn recorded_value(repo: &Repository, revision: &str, template: &str) -> anyhow::Result<String> {
    repo.jj(&[
        "--ignore-working-copy",
        "--at-op=@",
        "log",
        "--no-graph",
        "-r",
        revision,
        "-T",
        template,
    ])
}

fn recorded_parents(repo: &Repository, revision: &str) -> anyhow::Result<Vec<String>> {
    let output = recorded_value(
        repo,
        revision,
        r#"parents.map(|p| p.change_id()).join("\n")"#,
    )?;
    let mut parents = output.lines().map(str::to_owned).collect::<Vec<_>>();
    parents.sort();
    Ok(parents)
}

#[test]
fn workflow_base_pins_bookmark_without_snapshot_and_survives_movement_and_restart()
-> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        fs::write(repo.project.join("baseline.txt"), "main baseline\n")?;
        repo.jj(&["describe", "-m", "Main baseline"])?;
        repo.jj(&["bookmark", "create", "main"])?;
        let commit = recorded_value(&repo, "@", "commit_id")?;
        let change = recorded_value(&repo, "@", "change_id")?;
        fs::write(repo.project.join("dirty.txt"), "Do not snapshot\n")?;
        let operation = repo.operation()?;
        let checkout = fs::read(repo.project.join(".jj/working_copy/tree_state"))?;
        let database = repo.directory.path().join("workflow.db");
        let mut storage = Storage::open(&database).await?;
        let source = BaseSource::Bookmark {
            name: "main".to_owned(),
        };
        let workflow = storage
            .create_workflow_from(&repo.project, "From main", &source)
            .await?;
        let base = workflow.base.as_ref().expect("saved base");
        assert_eq!(base.source, source);
        assert_eq!(base.revision.commit_id, commit);
        assert_eq!(base.revision.change_id, change);
        assert_eq!(base.repository_path, fs::canonicalize(&repo.project)?);
        assert_eq!(
            repo.operation()?,
            operation,
            "selection must not snapshot or write native history"
        );
        assert_eq!(
            fs::read(repo.project.join(".jj/working_copy/tree_state"))?,
            checkout
        );
        assert_eq!(
            fs::read_to_string(repo.project.join("dirty.txt"))?,
            "Do not snapshot\n"
        );
        repo.jj(&["new", "-m", "Main advanced"])?;
        repo.jj(&["bookmark", "set", "main", "-r", "@"])?;
        assert_ne!(recorded_value(&repo, "main", "commit_id")?, commit);
        assert!(
            storage
                .set_workflow_base(workflow.id, &source)
                .await
                .is_err()
        );
        drop(storage);
        let storage = Storage::open(&database).await?;
        assert_eq!(storage.workflow(workflow.id).await?, workflow);
        assert_eq!(
            storage.list_workflows(&repo.project).await?,
            vec![workflow.clone()]
        );
        assert_eq!(
            storage.workflow_graph(workflow.id).await?.workflow,
            workflow
        );
        Ok(())
    })
}

#[test]
fn workflow_bases_accept_alternate_bookmarks_and_revision_ids_without_guessing()
-> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        repo.jj(&["describe", "-m", "Feature base"])?;
        repo.jj(&["bookmark", "create", "feature/auth"])?;
        let commit = recorded_value(&repo, "@", "commit_id")?;
        let change = recorded_value(&repo, "@", "change_id")?;
        let mut storage = Storage::open(&repo.directory.path().join("workflow.db")).await?;
        for source in [
            BaseSource::Bookmark {
                name: "feature/auth".to_owned(),
            },
            BaseSource::Revision { id: commit.clone() },
            BaseSource::Revision {
                id: commit[..12].to_owned(),
            },
            BaseSource::Revision { id: change },
        ] {
            let workflow = storage
                .create_workflow_from(&repo.project, "Alternate", &source)
                .await?;
            let base = workflow.base.expect("saved base");
            assert_eq!(base.source, source);
            assert_eq!(base.revision.commit_id, commit);
        }
        let before = storage.list_workflows(&repo.project).await?;
        let operation = repo.operation()?;
        for source in [
            BaseSource::Bookmark {
                name: "missing".to_owned(),
            },
            BaseSource::Bookmark {
                name: "feature/*".to_owned(),
            },
            BaseSource::Bookmark {
                name: "feature/auth\") | all() | bookmarks(\"".to_owned(),
            },
            BaseSource::Bookmark {
                name: " \n".to_owned(),
            },
            BaseSource::Revision { id: "@".to_owned() },
            BaseSource::Revision {
                id: "all()".to_owned(),
            },
            BaseSource::Revision {
                id: "0000000000000000000000000000000000000001".to_owned(),
            },
        ] {
            assert!(
                storage
                    .create_workflow_from(&repo.project, "Invalid", &source)
                    .await
                    .is_err(),
                "{source:?}"
            );
            assert_eq!(storage.list_workflows(&repo.project).await?, before);
        }
        assert_eq!(repo.operation()?, operation);
        Ok(())
    })
}

#[test]
fn legacy_base_selection_is_one_time_metadata_only_and_preserves_graph_and_history()
-> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        repo.jj(&["bookmark", "create", "main"])?;
        let mut storage = Storage::open(&repo.directory.path().join("workflow.db")).await?;
        let workflow = storage.create_workflow(&repo.project, "Legacy").await?;
        assert!(workflow.base.is_none());
        let node = storage.create_node(workflow.id, "Keep assignment").await?;
        let graph = storage.workflow_graph(workflow.id).await?;
        let history = storage.history_status(&repo.project).await?;
        let operation = repo.operation()?;
        let source = BaseSource::Bookmark {
            name: "main".to_owned(),
        };
        assert!(storage.set_workflow_base(9999, &source).await.is_err());
        let based = storage.set_workflow_base(workflow.id, &source).await?;
        assert!(based.base.is_some());
        let after = storage.workflow_graph(workflow.id).await?;
        assert_eq!(after.nodes, graph.nodes);
        assert_eq!(after.connections, graph.connections);
        assert_eq!(storage.node(workflow.id, node.id).await?, node);
        assert_eq!(storage.history_status(&repo.project).await?, history);
        assert_eq!(repo.operation()?, operation);
        assert!(
            storage
                .set_workflow_base(workflow.id, &source)
                .await
                .is_err()
        );
        assert_eq!(storage.workflow(workflow.id).await?, based);
        Ok(())
    })
}

#[test]
fn workflow_base_selection_rejects_conflicted_bookmarks_without_saving() -> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        repo.jj(&["bookmark", "create", "main"])?;
        repo.jj(&["new", "-m", "Left"])?;
        let left = repo.value("change_id")?;
        repo.jj(&["new", "root()", "-m", "Right"])?;
        let right = repo.value("change_id")?;
        let operation = repo.operation()?;
        repo.jj(&["bookmark", "set", "main", "-r", &left])?;
        repo.jj(&[
            "--at-op",
            &operation,
            "bookmark",
            "set",
            "main",
            "-r",
            &right,
            "--allow-backwards",
        ])?;
        repo.jj(&["log", "--no-graph", "-r", "@", "-T", "commit_id"])?;
        let mut storage = Storage::open(&repo.directory.path().join("workflow.db")).await?;
        let error = storage
            .create_workflow_from(
                &repo.project,
                "Conflicted",
                &BaseSource::Bookmark {
                    name: "main".to_owned(),
                },
            )
            .await
            .expect_err("conflicted bookmark must fail");
        assert!(
            error.to_string().contains("missing or conflicted"),
            "{error}"
        );
        assert!(storage.list_workflows(&repo.project).await?.is_empty());
        Ok(())
    })
}

#[test]
fn native_abandon_reconnects_chain_and_removes_only_abandoned_contribution() -> anyhow::Result<()> {
    let repo = Repository::new()?;
    fs::write(repo.project.join("interfaces.txt"), "interfaces\n")?;
    repo.jj(&["describe", "-m", "Interfaces"])?;
    let interfaces = repo.value("change_id")?;
    let base_commit = repo.value("commit_id")?;
    repo.jj(&["new", "-m", "Authentication"])?;
    fs::write(repo.project.join("authentication.txt"), "authentication\n")?;
    repo.jj(&["describe", "-m", "Authentication"])?;
    let authentication = repo.value("change_id")?;
    repo.jj(&["new", "-m", "Review"])?;
    fs::write(repo.project.join("review.txt"), "independent review work\n")?;
    repo.jj(&["describe", "-m", "Review"])?;
    let review = repo.value("change_id")?;
    let old_review_commit = repo.value("commit_id")?;

    repo.jj(&[
        "--ignore-working-copy",
        "--at-op=@",
        "abandon",
        &authentication,
    ])?;
    assert_eq!(recorded_parents(&repo, &review)?, vec![interfaces.clone()]);
    assert_eq!(
        recorded_value(
            &repo,
            &format!("change_id(\"{authentication}\")"),
            "change_id"
        )?,
        ""
    );
    assert_ne!(
        recorded_value(&repo, &review, "commit_id")?,
        old_review_commit
    );
    assert_eq!(
        recorded_value(&repo, &interfaces, "commit_id")?,
        base_commit
    );
    let files = repo.jj(&[
        "--ignore-working-copy",
        "--at-op=@",
        "file",
        "list",
        "-r",
        &review,
    ])?;
    assert_eq!(files, "interfaces.txt\nreview.txt\n");
    assert_eq!(recorded_value(&repo, &review, "conflict")?, "false");
    Ok(())
}

#[test]
fn native_abandon_rebases_all_children_not_only_selected_workflow_branch() -> anyhow::Result<()> {
    let repo = Repository::new()?;
    repo.jj(&["describe", "-m", "Base"])?;
    let base = repo.value("change_id")?;
    repo.jj(&["new", "-m", "Shared upstream"])?;
    let upstream = repo.value("change_id")?;
    let mut children = Vec::new();
    for label in ["Authentication", "Images", "Outside this workflow"] {
        repo.jj(&["new", &upstream, "-m", label])?;
        children.push(repo.value("change_id")?);
    }
    repo.jj(&["--ignore-working-copy", "--at-op=@", "abandon", &upstream])?;
    for child in children {
        assert_eq!(recorded_parents(&repo, &child)?, vec![base.clone()]);
    }
    Ok(())
}

#[test]
fn native_abandon_merge_reconnects_child_to_both_parents() -> anyhow::Result<()> {
    let repo = Repository::new()?;
    repo.jj(&["describe", "-m", "Base"])?;
    let base = repo.value("change_id")?;
    repo.jj(&["new", &base, "-m", "Authentication"])?;
    let auth = repo.value("change_id")?;
    repo.jj(&["new", &base, "-m", "Images"])?;
    let images = repo.value("change_id")?;
    repo.jj(&["new", &auth, &images, "-m", "Combined"])?;
    let combined = repo.value("change_id")?;
    repo.jj(&["new", "-m", "Review"])?;
    let review = repo.value("change_id")?;
    repo.jj(&["--ignore-working-copy", "--at-op=@", "abandon", &combined])?;
    let mut expected = vec![auth, images];
    expected.sort();
    assert_eq!(recorded_parents(&repo, &review)?, expected);
    Ok(())
}

#[test]
fn native_abandon_merge_parent_retains_inherited_parent_even_when_redundant() -> anyhow::Result<()>
{
    let repo = Repository::new()?;
    repo.jj(&["describe", "-m", "Base"])?;
    let base = repo.value("change_id")?;
    repo.jj(&["new", &base, "-m", "Authentication"])?;
    let auth = repo.value("change_id")?;
    repo.jj(&["new", &base, "-m", "Images"])?;
    let images = repo.value("change_id")?;
    repo.jj(&["new", &auth, &images, "-m", "Combined"])?;
    let combined = repo.value("change_id")?;
    repo.jj(&["--ignore-working-copy", "--at-op=@", "abandon", &auth])?;
    // Native jj retains the inherited Base parent even though it is also an
    // ancestor of Images. The graph must reflect the actual native result.
    let mut expected = vec![base, images];
    expected.sort();
    assert_eq!(recorded_parents(&repo, &combined)?, expected);
    Ok(())
}

#[test]
fn native_abandon_records_dependent_child_conflicts_without_losing_identity() -> anyhow::Result<()>
{
    let repo = Repository::new()?;
    repo.jj(&["describe", "-m", "Base"])?;
    let base = repo.value("change_id")?;
    repo.jj(&["new", "-m", "Authentication"])?;
    fs::write(repo.project.join("auth.txt"), "authentication API\n")?;
    repo.jj(&["describe", "-m", "Authentication"])?;
    let auth = repo.value("change_id")?;
    repo.jj(&["new", "-m", "Review depends on authentication"])?;
    fs::write(
        repo.project.join("auth.txt"),
        "reviewed authentication API\n",
    )?;
    repo.jj(&["describe", "-m", "Review depends on authentication"])?;
    let review = repo.value("change_id")?;
    repo.jj(&["--ignore-working-copy", "--at-op=@", "abandon", &auth])?;
    assert_eq!(recorded_parents(&repo, &review)?, vec![base]);
    assert_eq!(recorded_value(&repo, &review, "change_id")?, review);
    assert_eq!(recorded_value(&repo, &review, "conflict")?, "true");
    Ok(())
}

#[test]
fn native_abandon_does_not_remove_or_clean_a_workspace_editing_the_change() -> anyhow::Result<()> {
    let repo = Repository::new()?;
    repo.jj(&["describe", "-m", "Base"])?;
    repo.jj(&["new", "-m", "Authentication"])?;
    let auth = repo.value("change_id")?;
    repo.jj(&["new", "-m", "Review"])?;
    let workspace = repo.directory.path().join("auth-workspace");
    repo.jj(&[
        "--ignore-working-copy",
        "--at-op=@",
        "workspace",
        "add",
        "--name",
        "auth",
        "--no-colocate",
        "--sparse-patterns",
        "full",
        "-r",
        &auth,
        workspace.to_str().expect("workspace path"),
    ])?;
    repo.jj_at(
        &workspace,
        &["--ignore-working-copy", "--at-op=@", "edit", &auth],
    )?;
    repo.jj_at(&workspace, &["workspace", "update-stale"])?;
    fs::write(workspace.join("unrecorded.txt"), "Do not erase\n")?;
    let checkout = fs::read(workspace.join(".jj/working_copy/tree_state"))?;
    repo.jj(&["--ignore-working-copy", "--at-op=@", "abandon", &auth])?;
    assert!(workspace.is_dir());
    assert_eq!(
        fs::read_to_string(workspace.join("unrecorded.txt"))?,
        "Do not erase\n"
    );
    assert_eq!(
        fs::read(workspace.join(".jj/working_copy/tree_state"))?,
        checkout
    );
    let replacement = repo.jj_at(
        &workspace,
        &[
            "--ignore-working-copy",
            "--at-op=@",
            "log",
            "--no-graph",
            "-r",
            "@",
            "-T",
            "change_id",
        ],
    )?;
    assert_ne!(
        replacement, auth,
        "jj gives an abandoned working copy a replacement change"
    );
    Ok(())
}

#[test]
fn workspace_verification_preserves_edits_and_rejects_stale_checkouts() -> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        fs::write(repo.project.join("base.txt"), "base\n")?;
        repo.jj(&["describe", "-m", "Base"])?;
        let base = repo.value("change_id")?;
        let mut storage = Storage::open(&repo.directory.path().join("workflows.db")).await?;
        let workflow = storage
            .create_workflow(&repo.project, "Verification")
            .await?;
        let node = storage.create_node(workflow.id, "Keep prompt").await?;
        assert!(
            storage
                .verify_node_workspace(workflow.id, node.id)
                .await
                .is_err()
        );
        storage
            .create_node_change(workflow.id, node.id, &base)
            .await?;
        let path = repo.directory.path().join("editing");
        let workspace = storage
            .create_node_workspace(workflow.id, node.id, &path)
            .await?;
        // Unrelated repository operations do not make this checkout stale.
        let other = storage.create_node(workflow.id, "Independent").await?;
        storage
            .create_node_change(workflow.id, other.id, &base)
            .await?;
        fs::write(path.join("base.txt"), "legitimate local modification\n")?;
        fs::write(path.join("new.txt"), "not snapshotted\n")?;
        let operation = repo.operation()?;
        let checkout = fs::read(path.join(".jj/working_copy/checkout"))?;
        let tree = fs::read(path.join(".jj/working_copy/tree_state"))?;
        assert_eq!(
            storage.verify_node_workspace(workflow.id, node.id).await?,
            workspace
        );
        assert_eq!(repo.operation()?, operation);
        assert_eq!(fs::read(path.join(".jj/working_copy/checkout"))?, checkout);
        assert_eq!(fs::read(path.join(".jj/working_copy/tree_state"))?, tree);
        assert_eq!(
            fs::read_to_string(path.join("base.txt"))?,
            "legitimate local modification\n"
        );
        assert_eq!(
            fs::read_to_string(path.join("new.txt"))?,
            "not snapshotted\n"
        );
        // A local deletion is also legitimate; verification must not restore it.
        fs::remove_file(path.join("base.txt"))?;
        assert_eq!(
            storage.verify_node_workspace(workflow.id, node.id).await?,
            workspace
        );
        assert!(!path.join("base.txt").exists());
        let saved = workspace.workspace.expect("saved allocation");
        repo.jj(&[
            "--ignore-working-copy",
            "--at-op=@",
            "describe",
            "-r",
            &saved.change_id,
            "-m",
            "External rewrite",
        ])?;
        let operation = repo.operation()?;
        let before = storage.node_workspace(workflow.id, node.id).await?;
        let error = storage
            .verify_node_workspace(workflow.id, node.id)
            .await
            .expect_err("stale checkout must fail");
        assert!(
            error
                .to_string()
                .contains("checkout is unfinished or stale"),
            "{error}"
        );
        assert_eq!(storage.node_workspace(workflow.id, node.id).await?, before);
        assert_eq!(repo.operation()?, operation);
        assert_eq!(fs::read(path.join(".jj/working_copy/checkout"))?, checkout);
        assert_eq!(fs::read(path.join(".jj/working_copy/tree_state"))?, tree);
        assert!(!path.join("base.txt").exists());
        assert_eq!(
            fs::read_to_string(path.join("new.txt"))?,
            "not snapshotted\n"
        );
        assert!(storage.verify_node_workspace(99999, node.id).await.is_err());
        Ok(())
    })
}

#[test]
fn workspaces_edit_node_changes_independently_and_survive_restart() -> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        fs::write(repo.project.join("interfaces.txt"), "shared interface\n")?;
        repo.jj(&["describe", "-m", "Interfaces"])?;
        let base = repo.value("change_id")?;
        let original = repo.value("commit_id")?;
        let database = repo.directory.path().join("workflows.db");
        let mut storage = Storage::open(&database).await?;
        let workflow = storage
            .create_workflow(&repo.project, "Parallel work")
            .await?;
        let auth = storage.create_node(workflow.id, "Authentication").await?;
        let images = storage.create_node(workflow.id, "Image uploads").await?;
        let auth_change = storage
            .create_node_change(workflow.id, auth.id, &base)
            .await?;
        let image_change = storage
            .create_node_change(workflow.id, images.id, &base)
            .await?;
        assert!(matches!(
            storage.node_workspace(workflow.id, auth.id).await?.state,
            WorkspaceState::Unallocated
        ));
        fs::write(repo.project.join("dirty.txt"), "original dirty contents\n")?;
        let count = repo.jj(&["--ignore-working-copy", "log", "-r", "all()", "--count"])?;
        let auth_path = repo.directory.path().join("auth");
        let images_path = repo.directory.path().join("images");
        let auth_workspace = storage
            .create_node_workspace(workflow.id, auth.id, &auth_path)
            .await?;
        let image_workspace = storage
            .create_node_workspace(workflow.id, images.id, &images_path)
            .await?;
        assert!(
            matches!(auth_workspace.state, WorkspaceState::Ready { .. }),
            "{auth_workspace:?}"
        );
        assert!(
            matches!(image_workspace.state, WorkspaceState::Ready { .. }),
            "{image_workspace:?}"
        );
        assert_eq!(auth_workspace.node, auth);
        assert_eq!(
            fs::read_to_string(auth_path.join("interfaces.txt"))?,
            "shared interface\n"
        );
        assert_eq!(
            fs::read_to_string(images_path.join("interfaces.txt"))?,
            "shared interface\n"
        );
        assert_eq!(
            repo.jj(&["--ignore-working-copy", "log", "-r", "all()", "--count"])?,
            count,
            "workspace allocation must not leave extra child changes"
        );
        assert_eq!(
            repo.jj(&[
                "--ignore-working-copy",
                "log",
                "--no-graph",
                "-r",
                "@",
                "-T",
                "commit_id"
            ])?,
            original
        );
        assert_eq!(
            fs::read_to_string(repo.project.join("dirty.txt"))?,
            "original dirty contents\n"
        );
        assert!(!auth_path.join("dirty.txt").exists());
        let operation = repo.operation()?;
        assert_eq!(
            storage
                .create_node_workspace(workflow.id, auth.id, &auth_path)
                .await?,
            auth_workspace
        );
        assert!(
            storage
                .create_node_workspace(workflow.id, auth.id, &repo.directory.path().join("other"))
                .await
                .is_err()
        );
        assert_eq!(
            repo.operation()?,
            operation,
            "repeat requests must be observational"
        );
        fs::write(
            auth_path.join("auth.txt"),
            "authentication implementation\n",
        )?;
        // Explicit native jj use records edits. Waypoints status/refresh never snapshots.
        repo.jj_at(&auth_path, &["status"])?;
        let updated = storage.refresh_node(workflow.id, auth.id).await?;
        assert_eq!(updated.association, auth_change.association);
        assert_ne!(updated.state, auth_change.state);
        assert_eq!(
            storage.refresh_node(workflow.id, images.id).await?,
            image_change
        );
        assert!(!images_path.join("auth.txt").exists());
        assert!(!repo.project.join("auth.txt").exists());
        assert_eq!(
            repo.jj(&[
                "--ignore-working-copy",
                "log",
                "--no-graph",
                "-r",
                "@",
                "-T",
                "commit_id"
            ])?,
            original
        );
        let auth_status = storage.node_workspace(workflow.id, auth.id).await?;
        let operation = repo.operation()?;
        fs::write(auth_path.join("not-recorded.txt"), "keep dirty\n")?;
        assert_eq!(
            storage.node_workspace(workflow.id, auth.id).await?,
            auth_status
        );
        assert_eq!(repo.operation()?, operation);
        drop(storage);
        let storage = Storage::open(&database).await?;
        assert_eq!(
            storage.node_workspace(workflow.id, auth.id).await?,
            auth_status
        );
        assert_eq!(
            storage.node_workspace(workflow.id, images.id).await?,
            image_workspace
        );
        Ok(())
    })
}

#[test]
fn workspace_creation_rejects_unsafe_destinations_and_shared_changes() -> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        let mut storage = Storage::open(&repo.directory.path().join("workflows.db")).await?;
        let workflow = storage.create_workflow(&repo.project, "Validation").await?;
        let node = storage.create_node(workflow.id, "Prompt").await?;
        let destination = repo.directory.path().join("new");
        assert!(
            storage
                .create_node_workspace(workflow.id, node.id, &destination)
                .await
                .is_err()
        );
        assert!(!destination.exists());
        let original = repo.value("change_id")?;
        storage
            .associate_node(workflow.id, node.id, &original)
            .await?;
        let operation = repo.operation()?;
        assert!(
            storage
                .create_node_workspace(workflow.id, node.id, &destination)
                .await
                .is_err(),
            "must not edit the original workspace's change"
        );
        assert_eq!(repo.operation()?, operation);
        let node = storage.create_node(workflow.id, "Dedicated").await?;
        storage
            .create_node_change(workflow.id, node.id, "root()")
            .await?;
        let operation_after_setup = repo.operation()?;
        let occupied = repo.directory.path().join("occupied");
        fs::create_dir(&occupied)?;
        let file = repo.directory.path().join("file");
        fs::write(&file, "preserved")?;
        for invalid in [
            occupied.clone(),
            file.clone(),
            repo.project.join("nested"),
            repo.directory.path().join("missing-parent").join("new"),
            PathBuf::from("relative"),
        ] {
            assert!(
                storage
                    .create_node_workspace(workflow.id, node.id, &invalid)
                    .await
                    .is_err(),
                "{invalid:?}"
            );
        }
        assert!(
            storage
                .create_node_workspace(99999, node.id, &destination)
                .await
                .is_err()
        );
        assert_eq!(fs::read_to_string(file)?, "preserved");
        assert_eq!(fs::read_dir(occupied)?.count(), 0);
        #[cfg(unix)]
        {
            let link = repo.directory.path().join("dangling");
            std::os::unix::fs::symlink(repo.directory.path().join("absent"), &link)?;
            assert!(
                storage
                    .create_node_workspace(workflow.id, node.id, &link)
                    .await
                    .is_err()
            );
            assert!(fs::symlink_metadata(link)?.file_type().is_symlink());
        }
        let immutable = storage.create_node(workflow.id, "Immutable").await?;
        storage
            .associate_node(
                workflow.id,
                immutable.id,
                "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
            )
            .await?;
        assert!(
            storage
                .create_node_workspace(workflow.id, immutable.id, &destination)
                .await
                .is_err()
        );
        assert!(matches!(
            storage
                .node_workspace(workflow.id, immutable.id)
                .await?
                .state,
            WorkspaceState::Unallocated
        ));
        assert!(!destination.exists());
        assert_eq!(repo.operation()?, operation_after_setup);
        assert_ne!(operation_after_setup, operation);
        assert!(matches!(
            storage.node_workspace(workflow.id, node.id).await?.state,
            WorkspaceState::Unallocated
        ));
        storage
            .create_node_workspace(workflow.id, node.id, &destination)
            .await?;
        Ok(())
    })
}

#[test]
fn workspace_status_retains_missing_forgotten_and_switched_allocations() -> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        let mut storage = Storage::open(&repo.directory.path().join("workflows.db")).await?;
        let workflow = storage
            .create_workflow(&repo.project, "External changes")
            .await?;
        let node = storage.create_node(workflow.id, "Prompt").await?;
        storage
            .create_node_change(workflow.id, node.id, "root()")
            .await?;
        let path = repo.directory.path().join("workspace");
        let workspace = storage
            .create_node_workspace(workflow.id, node.id, &path)
            .await?;
        repo.jj_at(&path, &["new"])?;
        let operation = repo.operation()?;
        let switched = storage.node_workspace(workflow.id, node.id).await?;
        assert!(matches!(switched.state, WorkspaceState::Mismatched { .. }));
        assert_eq!(switched.workspace, workspace.workspace);
        assert!(
            storage
                .verify_node_workspace(workflow.id, node.id)
                .await
                .is_err()
        );
        assert_eq!(
            storage
                .create_node_workspace(workflow.id, node.id, &path)
                .await?,
            switched
        );
        assert_eq!(repo.operation()?, operation);
        let saved = workspace.workspace.expect("saved workspace");
        let original = repo.jj(&[
            "--ignore-working-copy",
            "log",
            "--no-graph",
            "-r",
            "@",
            "-T",
            "change_id",
        ])?;
        storage
            .associate_node(workflow.id, node.id, &original)
            .await?;
        let reassociated = storage.node_workspace(workflow.id, node.id).await?;
        assert!(matches!(
            reassociated.state,
            WorkspaceState::Mismatched { .. }
        ));
        assert_eq!(reassociated.workspace.as_ref(), Some(&saved));
        storage
            .associate_node(workflow.id, node.id, &saved.change_id)
            .await?;
        repo.jj(&["--ignore-working-copy", "workspace", "forget", &saved.name])?;
        assert!(matches!(
            storage.node_workspace(workflow.id, node.id).await?.state,
            WorkspaceState::Mismatched { .. }
        ));
        let moved = repo.directory.path().join("moved");
        fs::rename(&path, &moved)?;
        let missing = storage.node_workspace(workflow.id, node.id).await?;
        assert!(matches!(missing.state, WorkspaceState::MissingDirectory));
        assert!(
            storage
                .verify_node_workspace(workflow.id, node.id)
                .await
                .is_err()
        );
        assert_eq!(missing.workspace, Some(saved));
        assert_eq!(
            storage
                .create_node_workspace(workflow.id, node.id, &path)
                .await?,
            missing
        );
        assert!(
            !path.exists(),
            "repeat must not recreate a removed directory"
        );
        Ok(())
    })
}

#[test]
fn creation_preserves_working_copy_and_persists_explicit_sibling_ancestry() -> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        fs::write(repo.project.join("base.txt"), "base\n")?;
        let base = repo.value("change_id")?;
        let base_commit = repo.value("commit_id")?;
        repo.jj(&["new", &base])?;
        let working_copy = repo.value("commit_id")?;
        fs::write(repo.project.join("dirty.txt"), "not snapshotted\n")?;
        let database = repo.directory.path().join("workflows.db");
        let mut storage = Storage::open(&database).await?;
        let workflow = storage.create_workflow(&repo.project, "Create").await?;
        let first = storage
            .create_node(workflow.id, "Authentication\n日本語")
            .await?;
        let second = storage.create_node(workflow.id, "Images").await?;
        storage
            .connect_nodes(workflow.id, first.id, second.id)
            .await?;
        let created = storage
            .create_node_change(workflow.id, first.id, &base[..12])
            .await?;
        assert_eq!(created.node, first);
        let id = &created.association.as_ref().expect("association").change_id;
        assert_ne!(id, &base);
        assert_eq!(
            repo.jj(&[
                "--ignore-working-copy",
                "log",
                "--no-graph",
                "-r",
                "@",
                "-T",
                "commit_id"
            ])?,
            working_copy
        );
        assert_eq!(
            fs::read_to_string(repo.project.join("dirty.txt"))?,
            "not snapshotted\n"
        );
        assert_eq!(
            repo.jj(&[
                "--ignore-working-copy",
                "log",
                "--no-graph",
                "-r",
                id,
                "-T",
                "parents.map(|p| p.commit_id()).join(\" \") ++ \" \" ++ empty"
            ])?,
            format!("{base_commit} true")
        );
        let sibling = storage
            .create_node_change(workflow.id, second.id, &base_commit)
            .await?;
        let sibling_id = &sibling
            .association
            .as_ref()
            .expect("sibling association")
            .change_id;
        assert_eq!(
            repo.jj(&[
                "--ignore-working-copy",
                "log",
                "--no-graph",
                "-r",
                sibling_id,
                "-T",
                "parents.map(|p| p.commit_id()).join(\" \")"
            ])?,
            base_commit
        );
        let operation = repo.operation()?;
        assert!(
            storage
                .create_node_change(workflow.id, first.id, &base)
                .await
                .is_err()
        );
        assert_eq!(repo.operation()?, operation);
        drop(storage);
        let mut storage = Storage::open(&database).await?;
        assert_eq!(storage.refresh_node(workflow.id, first.id).await?, created);
        repo.jj(&["--ignore-working-copy", "abandon", id])?;
        let operation = repo.operation()?;
        assert!(
            storage
                .create_node_change(workflow.id, first.id, &base)
                .await
                .is_err()
        );
        assert_eq!(repo.operation()?, operation);
        assert!(matches!(
            storage.refresh_node(workflow.id, first.id).await?.state,
            ChangeState::Missing
        ));
        Ok(())
    })
}

#[test]
fn creation_rejects_invalid_bases_and_membership_before_mutating_jj() -> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        let mut storage = Storage::open(&repo.directory.path().join("workflows.db")).await?;
        let workflow = storage.create_workflow(&repo.project, "Validation").await?;
        let node = storage.create_node(workflow.id, "Prompt").await?;
        let operation = repo.operation()?;
        for base in [
            "",
            "@",
            "all()",
            "--help",
            "x | root()",
            "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdead",
            "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy",
        ] {
            assert!(
                storage
                    .create_node_change(workflow.id, node.id, base)
                    .await
                    .is_err(),
                "{base}"
            );
            assert_eq!(repo.operation()?, operation);
        }
        assert!(
            storage
                .create_node_change(99999, node.id, "root()")
                .await
                .is_err()
        );
        assert_eq!(repo.operation()?, operation);
        storage
            .create_node_change(workflow.id, node.id, "root()")
            .await?;
        Ok(())
    })
}

#[test]
fn associations_follow_rewrites_survive_restart_and_retain_missing_nodes() -> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        repo.jj(&["describe", "-m", "Initial description"])?;
        let id = repo.value("change_id")?;
        let database = repo.directory.path().join("workflows.db");
        let mut storage = Storage::open(&database).await?;
        // A project can be a subdirectory of a jj workspace.
        let project = repo.project.join("nested");
        fs::create_dir(&project)?;
        let workflow = storage
            .create_workflow(&project, "Native association")
            .await?;
        let other_workflow = storage.create_workflow(&project, "Independent").await?;
        let node = storage.create_node(workflow.id, "Keep this prompt").await?;
        assert!(matches!(
            storage.refresh_node(workflow.id, node.id).await?.state,
            ChangeState::Unassociated
        ));
        // A dirty file must not be snapshotted by association or refresh.
        fs::write(repo.project.join("unsnapshotted.txt"), "dirty contents")?;
        let operation = repo.operation()?;
        let selected = storage
            .associate_node(workflow.id, node.id, &id[..12])
            .await?;
        assert_eq!(
            selected
                .association
                .as_ref()
                .expect("saved association")
                .change_id,
            id
        );
        assert_eq!(
            selected
                .association
                .as_ref()
                .expect("saved association")
                .repository_path,
            repo.project.canonicalize()?
        );
        assert_eq!(
            repo.operation()?,
            operation,
            "selection must not snapshot or write an operation"
        );
        let initial_commit = match selected.state {
            ChangeState::Current { revision } => {
                assert_eq!(revision.description, "Initial description\n");
                assert!(!revision.conflicted);
                revision.commit_id
            }
            state => panic!("expected current, got {state:?}"),
        };
        assert_eq!(storage.refresh_node(workflow.id, node.id).await?.node, node);
        assert_eq!(repo.operation()?, operation);

        repo.jj(&[
            "describe",
            "-m",
            "Updated\nwith \"quotes\" and unicode 日本語",
        ])?;
        let operation = repo.operation()?;
        let refreshed = storage.refresh_node(workflow.id, node.id).await?;
        match &refreshed.state {
            ChangeState::Current { revision } => {
                assert_ne!(revision.commit_id, initial_commit);
                assert_eq!(revision.change_id, id);
                assert_eq!(
                    revision.description,
                    "Updated\nwith \"quotes\" and unicode 日本語\n"
                );
            }
            state => panic!("expected current, got {state:?}"),
        }
        assert_eq!(repo.operation()?, operation);
        assert!(
            storage
                .refresh_node(other_workflow.id, node.id)
                .await
                .is_err()
        );
        assert!(
            storage
                .associate_node(other_workflow.id, node.id, &id)
                .await
                .is_err()
        );
        for invalid in [
            "@",
            "all()",
            "--help",
            "",
            "abcdef",
            "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy",
        ] {
            assert!(
                storage
                    .associate_node(workflow.id, node.id, invalid)
                    .await
                    .is_err(),
                "{invalid:?}"
            );
            assert_eq!(storage.refresh_node(workflow.id, node.id).await?, refreshed);
        }
        drop(storage);
        let mut storage = Storage::open(&database).await?;
        assert_eq!(storage.refresh_node(workflow.id, node.id).await?, refreshed);
        repo.jj(&["abandon", "-r", &id])?;
        let operation = repo.operation()?;
        let missing = storage.refresh_node(workflow.id, node.id).await?;
        assert!(matches!(missing.state, ChangeState::Missing));
        assert_eq!(missing.association, refreshed.association);
        assert_eq!(missing.node, node);
        assert_eq!(repo.operation()?, operation);
        assert_eq!(
            storage.workflow_graph(workflow.id).await?.nodes,
            vec![node.clone()]
        );
        assert!(
            storage
                .associate_node(workflow.id, node.id, &id)
                .await
                .is_err()
        );
        assert_eq!(storage.refresh_node(workflow.id, node.id).await?, missing);
        let replacement = repo.value("change_id")?;
        let replaced = storage
            .associate_node(workflow.id, node.id, &replacement)
            .await?;
        assert_eq!(
            replaced.association.expect("new association").change_id,
            replacement
        );
        assert_eq!(replaced.node, node);
        Ok(())
    })
}

#[test]
fn repository_errors_are_not_missing_changes_and_do_not_destroy_associations() -> anyhow::Result<()>
{
    run(async {
        let repo = Repository::new()?;
        let id = repo.value("change_id")?;
        let mut storage = Storage::open(&repo.directory.path().join("workflows.db")).await?;
        let workflow = storage.create_workflow(&repo.project, "Repo").await?;
        let node = storage.create_node(workflow.id, "Prompt").await?;
        let selected = storage.associate_node(workflow.id, node.id, &id).await?;
        let moved = repo.directory.path().join("moved-project");
        fs::rename(&repo.project, &moved)?;
        assert!(storage.refresh_node(workflow.id, node.id).await.is_err());
        fs::rename(&moved, &repo.project)?;
        assert_eq!(storage.refresh_node(workflow.id, node.id).await?, selected);
        let plain = repo.directory.path().join("plain");
        fs::create_dir(&plain)?;
        let workflow = storage.create_workflow(&plain, "No jj").await?;
        let node = storage.create_node(workflow.id, "Unassociated").await?;
        assert!(matches!(
            storage.refresh_node(workflow.id, node.id).await?.state,
            ChangeState::Unassociated
        ));
        assert!(
            storage
                .create_node_change(workflow.id, node.id, "root()")
                .await
                .is_err()
        );
        assert!(
            storage
                .associate_node(workflow.id, node.id, &id)
                .await
                .is_err()
        );
        assert!(matches!(
            storage.refresh_node(workflow.id, node.id).await?.state,
            ChangeState::Unassociated
        ));
        Ok(())
    })
}

#[test]
fn refresh_reports_divergence_without_selecting_a_revision() -> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        repo.jj(&["describe", "-m", "Original"])?;
        let id = repo.value("change_id")?;
        let mut storage = Storage::open(&repo.directory.path().join("workflows.db")).await?;
        let workflow = storage.create_workflow(&repo.project, "Divergence").await?;
        let node = storage.create_node(workflow.id, "Prompt").await?;
        let selected = storage.associate_node(workflow.id, node.id, &id).await?;
        let operation = repo.operation()?;
        repo.jj(&["describe", "-r", &id, "-m", "First rewrite"])?;
        repo.jj(&[
            "--at-op",
            &operation,
            "describe",
            "-r",
            &id,
            "-m",
            "Second rewrite",
        ])?;
        // This explicit native operation integrates the two concurrent operation heads.
        repo.jj(&["status"])?;
        let operation = repo.operation()?;
        let refreshed = storage.refresh_node(workflow.id, node.id).await?;
        assert_eq!(refreshed.association, selected.association);
        match refreshed.state {
            ChangeState::Divergent { revisions } => {
                assert_eq!(revisions.len(), 2);
                assert!(revisions.iter().all(|revision| revision.change_id == id));
                assert_ne!(revisions[0].commit_id, revisions[1].commit_id);
            }
            state => panic!("expected divergence, got {state:?}"),
        }
        assert!(
            storage
                .associate_node(workflow.id, node.id, &id)
                .await
                .is_err()
        );
        let unassociated = storage
            .create_node(workflow.id, "Choose an exact base")
            .await?;
        assert!(
            storage
                .create_node_change(workflow.id, unassociated.id, &id)
                .await
                .is_err()
        );
        assert_eq!(repo.operation()?, operation);
        Ok(())
    })
}

#[test]
fn refresh_reads_native_conflict_status() -> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        fs::write(repo.project.join("file.txt"), "base\n")?;
        let base = repo.value("change_id")?;
        repo.jj(&["new", &base])?;
        fs::write(repo.project.join("file.txt"), "left\n")?;
        let left = repo.value("change_id")?;
        repo.jj(&["new", &base])?;
        fs::write(repo.project.join("file.txt"), "right\n")?;
        let right = repo.value("change_id")?;
        repo.jj(&["new", &left, &right])?;
        let merge = repo.value("change_id")?;
        let mut storage = Storage::open(&repo.directory.path().join("workflows.db")).await?;
        let workflow = storage.create_workflow(&repo.project, "Conflict").await?;
        let node = storage.create_node(workflow.id, "Resolve later").await?;
        let selected = storage.associate_node(workflow.id, node.id, &merge).await?;
        match selected.state {
            ChangeState::Current { revision } => assert!(revision.conflicted),
            state => panic!("expected conflicted current revision, got {state:?}"),
        }
        Ok(())
    })
}
