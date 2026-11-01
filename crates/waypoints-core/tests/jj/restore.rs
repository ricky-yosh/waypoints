use super::*;
use waypoints_core::run::{Run, RunState, SnapshotState};
use waypoints_core::storage::RestoreState;

async fn fixture() -> anyhow::Result<(Repository, Storage, Workflow, Node, PathBuf)> {
    let repo = Repository::new()?;
    let mut storage = Storage::open(&repo.directory.path().join("restore.db")).await?;
    let workflow = storage
        .create_workflow_from(
            &repo.project,
            "Restore",
            &BaseSource::Revision {
                id: "root()".into(),
            },
        )
        .await?;
    let node = storage
        .create_node_from(workflow.id, "Build login", &NodeParent::WorkflowBase {})
        .await?;
    storage
        .create_node_change_from_dependency(workflow.id, node.id)
        .await?;
    let workspace = repo.directory.path().join("workspace");
    storage
        .create_node_workspace(workflow.id, node.id, &workspace)
        .await?;
    // Set a repository-local identity so diagnostic-free snapshot checks do not
    // depend on the test runner's user configuration.
    repo.jj_at(
        &workspace,
        &["config", "set", "--repo", "user.name", "Restore test"],
    )?;
    repo.jj_at(
        &workspace,
        &[
            "config",
            "set",
            "--repo",
            "user.email",
            "test@example.invalid",
        ],
    )?;
    Ok((repo, storage, workflow, node, workspace))
}

async fn saved_run(
    storage: &mut Storage,
    workflow: &Workflow,
    node: &Node,
    session: &str,
) -> anyhow::Result<Run> {
    let mut run = storage
        .create_run(workflow.id, node.id, Path::new("/bin/sh"))
        .await?;
    run.session_id = Some(session.into());
    run.state = RunState::Finalizing;
    storage.save_run(&run).await?;
    let revision = storage.prepare_run_snapshot(run.id).await?.record().await?;
    run.snapshot = SnapshotState::Saved { revision };
    run.state = RunState::Completed;
    storage.save_run(&run).await?;
    Ok(run)
}

#[test]
fn restore_preserves_dirty_work_same_change_and_history_then_requires_a_fresh_session()
-> anyhow::Result<()> {
    run(async {
        let (repo, mut storage, workflow, node, workspace) = fixture().await?;
        fs::write(workspace.join("login.txt"), "basic form\n")?;
        let first = saved_run(&mut storage, &workflow, &node, "first-session").await?;
        fs::write(workspace.join("login.txt"), "animations\n")?;
        let second = saved_run(&mut storage, &workflow, &node, "second-session").await?;
        let first_review = storage.review_run(first.id).await?;
        let second_review = storage.review_run(second.id).await?;
        fs::write(workspace.join("login.txt"), "unsaved latest edits\n")?;
        fs::write(workspace.join("new.txt"), "not lost\n")?;
        fs::write(repo.project.join("original-dirty.txt"), "leave alone\n")?;
        let original = recorded_value(&repo, "@", "commit_id")?;
        let graph = storage.workflow_graph(workflow.id).await?;
        let history = storage.history_status(&repo.project).await?;
        let restored = storage.restore_run(first.id).await?;
        assert_eq!(restored.state, RestoreState::Completed);
        assert_eq!(restored.source, first_review.revision);
        assert_eq!(restored.after.as_ref().unwrap().change_id, first.change_id);
        assert_ne!(
            restored.after.as_ref().unwrap().commit_id,
            restored.source.commit_id
        );
        assert_eq!(
            fs::read_to_string(workspace.join("login.txt"))?,
            "basic form\n"
        );
        assert!(!workspace.join("new.txt").exists());
        let preserved = restored.preserved.as_ref().unwrap();
        assert_eq!(
            repo.jj_at(
                &workspace,
                &[
                    "--ignore-working-copy",
                    "file",
                    "show",
                    "-r",
                    &preserved.commit_id,
                    "login.txt",
                ]
            )?,
            "unsaved latest edits\n"
        );
        assert_eq!(
            repo.jj_at(
                &workspace,
                &[
                    "--ignore-working-copy",
                    "file",
                    "show",
                    "-r",
                    &preserved.commit_id,
                    "new.txt",
                ]
            )?,
            "not lost\n"
        );
        // Native evolution retains the later result and the pre-restore checkpoint.
        let evolution = repo.jj_at(
            &workspace,
            &[
                "--ignore-working-copy",
                "evolog",
                "--no-graph",
                "-r",
                "@",
                "-T",
                "commit.commit_id()",
            ],
        )?;
        assert!(evolution.contains(&preserved.commit_id), "{evolution}");
        assert!(
            evolution.contains(&second_review.revision.commit_id),
            "{evolution}"
        );
        assert_eq!(recorded_value(&repo, "@", "commit_id")?, original);
        assert_eq!(
            fs::read_to_string(repo.project.join("original-dirty.txt"))?,
            "leave alone\n"
        );
        assert_eq!(storage.workflow_graph(workflow.id).await?, graph);
        assert_eq!(storage.history_status(&repo.project).await?, history);
        assert_eq!(storage.review_run(first.id).await?, first_review);
        assert_eq!(storage.review_run(second.id).await?, second_review);
        assert!(
            storage
                .continue_run(first.id, "wrong history")
                .await
                .is_err()
        );
        assert!(
            storage
                .continue_run(second.id, "wrong history")
                .await
                .is_err()
        );
        drop(storage);
        let mut storage = Storage::open(&repo.directory.path().join("restore.db")).await?;
        assert_eq!(storage.run_restores(first.id).await?, [restored]);
        assert!(
            storage
                .continue_run(second.id, "still wrong")
                .await
                .is_err()
        );
        let fresh = saved_run(&mut storage, &workflow, &node, "fresh-session").await?;
        let mut next = storage
            .continue_run(fresh.id, "continue fresh history")
            .await?;
        assert_eq!(next.session_id.as_deref(), Some("fresh-session"));
        next.state = RunState::Cancelled;
        storage.save_run(&next).await?;
        Ok(())
    })
}

#[test]
fn restore_refuses_busy_missing_snapshot_and_saved_or_native_descendants_without_mutation()
-> anyhow::Result<()> {
    run(async {
        let (repo, mut storage, workflow, node, workspace) = fixture().await?;
        fs::write(workspace.join("login.txt"), "saved\n")?;
        let saved = saved_run(&mut storage, &workflow, &node, "saved-session").await?;
        let mut busy = storage
            .create_run(workflow.id, node.id, Path::new("/bin/sh"))
            .await?;
        let before = repo.operation()?;
        fs::write(workspace.join("dirty.txt"), "preserve\n")?;
        assert!(storage.restore_run(saved.id).await.is_err());
        busy.state = RunState::Cancelled;
        storage.save_run(&busy).await?;
        assert!(storage.restore_run(busy.id).await.is_err());
        assert_eq!(repo.operation()?, before);
        let child = storage
            .create_node_from(
                workflow.id,
                "Downstream",
                &NodeParent::Node { node_id: node.id },
            )
            .await?;
        assert!(storage.restore_run(saved.id).await.is_err());
        storage.delete_prompt_node(workflow.id, child.id).await?;
        assert_eq!(repo.operation()?, before);
        repo.jj(&["new", &saved.change_id, "--no-edit"])?;
        let native = repo.operation()?;
        assert!(storage.restore_run(saved.id).await.is_err());
        assert_eq!(repo.operation()?, native);
        assert_eq!(
            fs::read_to_string(workspace.join("dirty.txt"))?,
            "preserve\n"
        );
        assert!(storage.run_restores(saved.id).await?.is_empty());
        Ok(())
    })
}

#[test]
fn restore_refuses_stale_checkout_and_different_parent_revision() -> anyhow::Result<()> {
    run(async {
        let (repo, mut storage, workflow, node, workspace) = fixture().await?;
        fs::write(workspace.join("login.txt"), "saved\n")?;
        let saved = saved_run(&mut storage, &workflow, &node, "session").await?;
        repo.jj(&[
            "--ignore-working-copy",
            "describe",
            "-r",
            &saved.change_id,
            "-m",
            "external edit",
        ])?;
        let operation = repo.operation()?;
        assert!(storage.restore_run(saved.id).await.is_err());
        assert_eq!(repo.operation()?, operation);
        repo.jj_at(&workspace, &["workspace", "update-stale"])?;
        // Rewrite onto another parent without changing this saved run.
        repo.jj(&["new", "root()"])?;
        let parent = repo.value("commit_id")?;
        repo.jj(&[
            "--ignore-working-copy",
            "rebase",
            "-r",
            &saved.change_id,
            "-d",
            &parent,
        ])?;
        repo.jj_at(&workspace, &["workspace", "update-stale"])?;
        let operation = repo.operation()?;
        let error = storage.restore_run(saved.id).await.unwrap_err().to_string();
        assert!(error.contains("different parents"), "{error}");
        assert_eq!(repo.operation()?, operation);
        assert!(storage.run_restores(saved.id).await?.is_empty());
        Ok(())
    })
}
