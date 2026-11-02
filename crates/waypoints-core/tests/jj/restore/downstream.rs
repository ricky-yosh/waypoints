use super::*;
use waypoints_core::storage::{RestoreImpact, RestoreRebaseConfirmation};

async fn chain() -> anyhow::Result<(
    Repository,
    Storage,
    Workflow,
    Node,
    Node,
    PathBuf,
    PathBuf,
    Run,
    Run,
)> {
    let (repo, mut storage, workflow, source, source_dir) = fixture().await?;
    fs::write(source_dir.join("api.txt"), "API A\n")?;
    let selected = saved_run(&mut storage, &workflow, &source, "source").await?;
    fs::write(source_dir.join("api.txt"), "API B\n")?;
    repo.jj_at(&source_dir, &["describe", "-m", "API B"])?;
    let child = storage
        .create_node_from(
            workflow.id,
            "Login",
            &NodeParent::Node { node_id: source.id },
        )
        .await?;
    storage
        .create_node_change_from_dependency(workflow.id, child.id)
        .await?;
    let child_dir = repo.directory.path().join("login");
    storage
        .create_node_workspace(workflow.id, child.id, &child_dir)
        .await?;
    fs::write(child_dir.join("login.txt"), "Login\n")?;
    let child_run = saved_run(&mut storage, &workflow, &child, "child").await?;
    Ok((
        repo, storage, workflow, source, child, source_dir, child_dir, selected, child_run,
    ))
}

async fn preview(storage: &Storage, selected: &Run) -> anyhow::Result<RestoreImpact> {
    let SnapshotState::Saved { revision } = &selected.snapshot else {
        panic!("saved run")
    };
    Ok(storage
        .node_restore_impact(selected.workflow_id, selected.node_id, &revision.commit_id)
        .await?)
}

#[test]
fn confirmed_restore_checkpoints_dirty_descendants_and_requires_fresh_sessions_even_when_clean()
-> anyhow::Result<()> {
    run(async {
        let (
            repo,
            mut storage,
            workflow,
            source,
            child,
            source_dir,
            child_dir,
            selected,
            child_run,
        ) = chain().await?;
        let prompt = storage
            .create_node_from(
                workflow.id,
                "Test login",
                &NodeParent::Node { node_id: child.id },
            )
            .await?;
        fs::write(source_dir.join("api.txt"), "unsaved API C\n")?;
        fs::write(child_dir.join("login.txt"), "unsaved login\n")?;
        fs::write(child_dir.join("extra.txt"), "preserve me\n")?;
        let graph = storage.workflow_graph(workflow.id).await?;
        let history = storage.history_status(&repo.project).await?;
        let expected = preview(&storage, &selected).await?;
        let native = repo.operation()?;
        assert!(
            storage
                .restore_node_with_descendants(&expected, None)
                .await
                .is_err()
        );
        assert_eq!(repo.operation()?, native);
        assert!(
            storage
                .node_restores(workflow.id, source.id)
                .await?
                .is_empty()
        );
        let restored = storage
            .restore_node_with_descendants(
                &expected,
                Some(RestoreRebaseConfirmation::RestoreAndRebase),
            )
            .await?;
        assert_eq!(restored.state, RestoreState::Completed);
        assert_eq!(
            restored.before.change_id,
            restored.after.as_ref().unwrap().change_id
        );
        assert_eq!(fs::read_to_string(source_dir.join("api.txt"))?, "API A\n");
        let scope = restored.downstream.as_ref().unwrap();
        assert_eq!(
            scope.nodes.iter().map(|n| n.node_id).collect::<Vec<_>>(),
            [prompt.id, child.id, source.id]
        );
        let child_result = scope.nodes.iter().find(|n| n.node_id == child.id).unwrap();
        assert!(child_result.needs_workspace_update);
        assert!(!child_result.after.as_ref().unwrap().conflicted);
        let checkpoint = child_result.preserved.as_ref().unwrap();
        assert_eq!(
            repo.jj(&[
                "--ignore-working-copy",
                "file",
                "show",
                "-r",
                &checkpoint.commit_id,
                "login.txt"
            ])?,
            "unsaved login\n"
        );
        assert_eq!(
            repo.jj(&[
                "--ignore-working-copy",
                "file",
                "show",
                "-r",
                &checkpoint.commit_id,
                "extra.txt"
            ])?,
            "preserve me\n"
        );
        assert_eq!(
            repo.jj(&[
                "--ignore-working-copy",
                "file",
                "show",
                "-r",
                &restored.preserved.as_ref().unwrap().commit_id,
                "api.txt"
            ])?,
            "unsaved API C\n"
        );
        assert_eq!(
            fs::read_to_string(child_dir.join("api.txt"))?,
            "API B\n",
            "no silent descendant checkout update"
        );
        assert_eq!(
            fs::read_to_string(child_dir.join("login.txt"))?,
            "unsaved login\n"
        );
        for run in [&selected, &child_run] {
            let error = storage
                .continue_run(run.id, "continue old memory")
                .await
                .unwrap_err();
            assert!(error.to_string().contains("fresh run"), "{error:#}");
            assert_eq!(storage.run(run.id).await?, *run);
        }
        assert_eq!(
            storage.node_restores(workflow.id, child.id).await?,
            std::slice::from_ref(&restored)
        );
        assert_eq!(
            storage.node_restores(workflow.id, prompt.id).await?,
            std::slice::from_ref(&restored)
        );
        assert_eq!(storage.workflow_graph(workflow.id).await?, graph);
        assert_eq!(storage.history_status(&repo.project).await?, history);
        assert!(
            storage
                .create_run(workflow.id, child.id, Path::new("/bin/sh"))
                .await
                .is_err()
        );
        let update = storage
            .update_stale_node_workspace(workflow.id, child.id)
            .await?;
        assert_eq!(
            update.outcome,
            waypoints_core::workspace::WorkspaceUpdateOutcome::Updated
        );
        assert_eq!(fs::read_to_string(child_dir.join("api.txt"))?, "API A\n");
        assert_eq!(
            fs::read_to_string(child_dir.join("login.txt"))?,
            "unsaved login\n"
        );
        drop(storage);
        let mut storage = Storage::open(&repo.directory.path().join("restore.db")).await?;
        assert_eq!(storage.restore_operation(restored.id).await?, restored);
        assert!(
            storage
                .continue_run(child_run.id, "still old")
                .await
                .is_err()
        );
        let fresh = storage
            .create_run(workflow.id, child.id, Path::new("/bin/sh"))
            .await?;
        assert_eq!(fresh.continued_from, None);
        assert_eq!(fresh.session_id, None);
        Ok(())
    })
}

#[test]
fn changed_preview_refuses_before_reservation_for_prompt_edges_native_history_and_sessions()
-> anyhow::Result<()> {
    run(async {
        let (repo, mut storage, workflow, source, child, source_dir, _, selected, _) =
            chain().await?;
        let old = preview(&storage, &selected).await?;
        storage
            .update_node(workflow.id, child.id, "Different assignment")
            .await?;
        let operation = repo.operation()?;
        assert!(
            storage
                .restore_node_with_descendants(
                    &old,
                    Some(RestoreRebaseConfirmation::RestoreAndRebase)
                )
                .await
                .is_err()
        );
        assert_eq!(repo.operation()?, operation);
        let old = preview(&storage, &selected).await?;
        repo.jj_at(&source_dir, &["describe", "-m", "external rewrite"])?;
        let operation = repo.operation()?;
        assert!(
            storage
                .restore_node_with_descendants(
                    &old,
                    Some(RestoreRebaseConfirmation::RestoreAndRebase)
                )
                .await
                .is_err()
        );
        assert_eq!(repo.operation()?, operation);
        let old = preview(&storage, &selected).await?;
        let mut active = storage
            .create_run(workflow.id, source.id, Path::new("/bin/sh"))
            .await?;
        assert!(
            storage
                .restore_node_with_descendants(
                    &old,
                    Some(RestoreRebaseConfirmation::RestoreAndRebase)
                )
                .await
                .is_err()
        );
        active.state = RunState::Cancelled;
        storage.save_run(&active).await?;
        assert!(
            storage
                .restore_node_with_descendants(
                    &old,
                    Some(RestoreRebaseConfirmation::RestoreAndRebase)
                )
                .await
                .is_err()
        );
        assert!(
            storage
                .node_restores(workflow.id, source.id)
                .await?
                .is_empty()
        );
        Ok(())
    })
}

#[test]
fn native_rebase_conflicts_are_visible_completed_results_not_silently_resolved()
-> anyhow::Result<()> {
    run(async {
        let (repo, mut storage, workflow, source, child, _, child_dir, selected, _) =
            chain().await?;
        fs::write(child_dir.join("api.txt"), "Child changed API B\n")?;
        let expected = preview(&storage, &selected).await?;
        let restored = storage
            .restore_node_with_descendants(
                &expected,
                Some(RestoreRebaseConfirmation::RestoreAndRebase),
            )
            .await?;
        assert_eq!(restored.state, RestoreState::Completed);
        let child_result = restored
            .downstream
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .find(|n| n.node_id == child.id)
            .unwrap();
        assert!(child_result.after.as_ref().unwrap().conflicted);
        assert_eq!(
            child_result.before.as_ref().unwrap().change_id,
            child_result.after.as_ref().unwrap().change_id
        );
        assert!(child_result.needs_workspace_update);
        assert_eq!(
            fs::read_to_string(child_dir.join("api.txt"))?,
            "Child changed API B\n"
        );
        assert!(
            storage
                .node_runs(workflow.id, source.id)
                .await?
                .iter()
                .all(|run| run.id == selected.id)
        );
        assert!(!repo.operation()?.is_empty());
        Ok(())
    })
}

#[test]
fn restore_rebases_a_merge_once_and_preserves_its_unrelated_input() -> anyhow::Result<()> {
    run(async {
        let (repo, mut storage, workflow, source, child, _, _, selected, _) = chain().await?;
        let other = storage
            .create_node_from(
                workflow.id,
                "Independent input",
                &NodeParent::WorkflowBase {},
            )
            .await?;
        storage
            .create_node_change_from_dependency(workflow.id, other.id)
            .await?;
        let other_dir = repo.directory.path().join("other");
        storage
            .create_node_workspace(workflow.id, other.id, &other_dir)
            .await?;
        fs::write(other_dir.join("other.txt"), "independent input\n")?;
        let unaffected = saved_run(&mut storage, &workflow, &other, "independent").await?;
        let other_revision = storage.review_node(workflow.id, other.id).await?.revision;
        let merge = storage
            .create_node_from_parents(
                workflow.id,
                "Integrate",
                &[
                    NodeParent::Node { node_id: child.id },
                    NodeParent::Node { node_id: other.id },
                ],
            )
            .await?;
        storage
            .create_node_change_from_dependency(workflow.id, merge.id)
            .await?;
        let merge_dir = repo.directory.path().join("merge");
        storage
            .create_node_workspace(workflow.id, merge.id, &merge_dir)
            .await?;
        fs::write(merge_dir.join("merge.txt"), "unsaved integration work\n")?;
        let expected = preview(&storage, &selected).await?;
        let result = storage
            .restore_node_with_descendants(
                &expected,
                Some(RestoreRebaseConfirmation::RestoreAndRebase),
            )
            .await?;
        let scope = result.downstream.as_ref().unwrap();
        assert_eq!(
            scope.nodes.iter().map(|n| n.node_id).collect::<Vec<_>>(),
            [merge.id, child.id, source.id]
        );
        let merged = scope.nodes.iter().find(|n| n.node_id == merge.id).unwrap();
        let review = storage.review_node(workflow.id, merge.id).await?;
        assert_eq!(review.parent_commit_ids.len(), 2);
        assert!(review.parent_commit_ids.contains(&other_revision.commit_id));
        assert_eq!(
            storage.review_node(workflow.id, other.id).await?.revision,
            other_revision
        );
        assert!(!merged.after.as_ref().unwrap().conflicted);
        assert_eq!(
            repo.jj(&[
                "--ignore-working-copy",
                "file",
                "show",
                "-r",
                &merged.preserved.as_ref().unwrap().commit_id,
                "merge.txt"
            ])?,
            "unsaved integration work\n"
        );
        assert_eq!(
            repo.jj(&[
                "--ignore-working-copy",
                "file",
                "show",
                "-r",
                &merged.after.as_ref().unwrap().commit_id,
                "other.txt"
            ])?,
            "independent input\n"
        );
        let continued = storage
            .continue_run(unaffected.id, "Follow up on unchanged input")
            .await?;
        assert_eq!(continued.continued_from, Some(unaffected.id));
        Ok(())
    })
}

#[test]
fn unsafe_descendants_refuse_before_reservation_or_snapshot() -> anyhow::Result<()> {
    run(async {
        for case in [
            "unmanaged",
            "cross_workflow",
            "ancestry",
            "stale",
            "missing",
            "shared",
            "immutable",
            "divergent",
        ] {
            let (
                repo,
                mut storage,
                workflow,
                source,
                child,
                source_dir,
                child_dir,
                selected,
                child_run,
            ) = chain().await?;
            let current = storage.review_node(workflow.id, source.id).await?.revision;
            let child_revision = storage.review_node(workflow.id, child.id).await?.revision;
            match case {
                "unmanaged" | "cross_workflow" => {
                    repo.jj(&["new", "--no-edit", &current.commit_id, "-m", "unmanaged"])?;
                    if case == "cross_workflow" {
                        let other = storage
                            .create_workflow(&repo.project, "Other workflow")
                            .await?;
                        let node = storage.create_node(other.id, "External descendant").await?;
                        let change = recorded_value(
                            &repo,
                            &format!(
                                "({}::) ~ {} ~ {}",
                                current.commit_id, current.commit_id, child_revision.commit_id
                            ),
                            "change_id",
                        )?;
                        storage.associate_node(other.id, node.id, &change).await?;
                    }
                }
                "ancestry" => {
                    let original = recorded_value(&repo, "@", "commit_id")?;
                    repo.jj_at(
                        &child_dir,
                        &["rebase", "-r", &child_revision.commit_id, "-d", &original],
                    )?;
                }
                "stale" => {
                    repo.jj(&[
                        "--ignore-working-copy",
                        "describe",
                        "-r",
                        &child_revision.commit_id,
                        "-m",
                        "external",
                    ])?;
                }
                "missing" => fs::rename(&child_dir, repo.directory.path().join("moved"))?,
                "shared" => {
                    let extra = repo.directory.path().join("extra");
                    repo.jj(&[
                        "workspace",
                        "add",
                        "--name",
                        "extra",
                        "-r",
                        &child_revision.commit_id,
                        extra.to_str().unwrap(),
                    ])?;
                    repo.jj_at(&extra, &["edit", &child_revision.commit_id])?;
                }
                "immutable" => {
                    repo.jj(&[
                        "bookmark",
                        "create",
                        "protected",
                        "-r",
                        &child_revision.commit_id,
                    ])?;
                    repo.jj(&[
                        "config",
                        "set",
                        "--repo",
                        "revset-aliases.\"immutable_heads()\"",
                        "protected",
                    ])?;
                }
                "divergent" => {
                    let operation = repo.operation()?;
                    repo.jj_at(&child_dir, &["describe", "-m", "one"])?;
                    repo.jj_at(
                        &child_dir,
                        &[
                            "--at-op",
                            &operation,
                            "--ignore-working-copy",
                            "describe",
                            "-r",
                            &child_revision.commit_id,
                            "-m",
                            "two",
                        ],
                    )?;
                    repo.jj(&["op", "log", "--limit", "1"])?;
                }
                _ => unreachable!(),
            }
            fs::write(source_dir.join("keep.txt"), "do not snapshot refusal\n")?;
            let expected = preview(&storage, &selected).await?;
            let operation = repo.operation()?;
            assert!(
                storage
                    .restore_node_with_descendants(
                        &expected,
                        Some(RestoreRebaseConfirmation::RestoreAndRebase)
                    )
                    .await
                    .is_err(),
                "{case}"
            );
            assert_eq!(repo.operation()?, operation, "{case}");
            assert_eq!(
                fs::read_to_string(source_dir.join("keep.txt"))?,
                "do not snapshot refusal\n"
            );
            assert!(
                storage
                    .node_restores(workflow.id, source.id)
                    .await?
                    .is_empty(),
                "{case}"
            );
            assert_eq!(storage.run(child_run.id).await?, child_run);
        }
        Ok(())
    })
}

#[test]
fn even_a_noop_restore_invalidates_its_old_confirmation_preview() -> anyhow::Result<()> {
    run(async {
        let (repo, mut storage, workflow, source, _) = fixture().await?;
        let current = storage.review_node(workflow.id, source.id).await?.revision;
        let expected = storage
            .node_restore_impact(workflow.id, source.id, &current.commit_id)
            .await?;
        let result = storage
            .restore_node_with_descendants(
                &expected,
                Some(RestoreRebaseConfirmation::RestoreAndRebase),
            )
            .await?;
        let refreshed = storage
            .node_restore_impact(workflow.id, source.id, &current.commit_id)
            .await?;
        assert!(refreshed.restore_watermark > expected.restore_watermark);
        assert_eq!(
            repo.operation()?,
            expected.native_operation,
            "exercise a native no-op, not only an ordinary stale operation"
        );
        let error = storage
            .restore_node_with_descendants(
                &expected,
                Some(RestoreRebaseConfirmation::RestoreAndRebase),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("preview changed"), "{error:#}");
        assert_eq!(
            storage.node_restores(workflow.id, source.id).await?,
            [result]
        );
        Ok(())
    })
}

#[test]
fn unallocated_native_descendants_keep_exact_checkpoints_without_creating_workspaces()
-> anyhow::Result<()> {
    run(async {
        let (repo, mut storage, workflow, _, child, _, _, selected, _) = chain().await?;
        let leaf = storage
            .create_node_from(
                workflow.id,
                "Unallocated follow-up",
                &NodeParent::Node { node_id: child.id },
            )
            .await?;
        storage
            .create_node_change_from_dependency(workflow.id, leaf.id)
            .await?;
        let before = storage.review_node(workflow.id, leaf.id).await?.revision;
        let expected = preview(&storage, &selected).await?;
        let result = storage
            .restore_node_with_descendants(
                &expected,
                Some(RestoreRebaseConfirmation::RestoreAndRebase),
            )
            .await?;
        let leaf_result = result
            .downstream
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .find(|n| n.node_id == leaf.id)
            .unwrap();
        assert_eq!(leaf_result.preserved.as_ref(), Some(&before));
        assert_eq!(
            leaf_result.after.as_ref().unwrap().change_id,
            before.change_id
        );
        assert!(leaf_result.workspace.is_none());
        assert!(!leaf_result.needs_workspace_update);
        assert!(
            storage
                .node_workspace(workflow.id, leaf.id)
                .await?
                .workspace
                .is_none()
        );
        assert_eq!(
            repo.jj(&[
                "--ignore-working-copy",
                "file",
                "show",
                "-r",
                &leaf_result.after.as_ref().unwrap().commit_id,
                "api.txt"
            ])?,
            "API A\n"
        );
        Ok(())
    })
}

#[test]
fn oversized_completion_is_refused_before_any_checkpoint_or_journal_reservation()
-> anyhow::Result<()> {
    run(async {
        let (repo, mut storage, workflow, source, _, source_dir, child_dir, selected, _) =
            chain().await?;
        repo.jj_at(&source_dir, &["describe", "-m", &"x".repeat(180_000)])?;
        repo.jj_at(&child_dir, &["workspace", "update-stale"])?;
        let expected = preview(&storage, &selected).await?;
        let operation = repo.operation()?;
        let error = storage
            .restore_node_with_descendants(
                &expected,
                Some(RestoreRebaseConfirmation::RestoreAndRebase),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("response limit"), "{error:#}");
        assert_eq!(repo.operation()?, operation);
        assert!(
            storage
                .node_restores(workflow.id, source.id)
                .await?
                .is_empty()
        );
        Ok(())
    })
}
