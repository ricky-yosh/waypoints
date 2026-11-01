use super::*;
use std::collections::BTreeSet;
use waypoints_core::storage::RestoreImpactCheckout;

fn workspace_commit(repo: &Repository, workspace: &Path) -> anyhow::Result<String> {
    repo.jj_at(
        workspace,
        &[
            "--ignore-working-copy",
            "log",
            "--no-graph",
            "-r",
            "@",
            "-T",
            "commit_id",
        ],
    )
}

#[test]
fn leaf_impact_is_readonly_including_dirty_files_sessions_and_restart() -> anyhow::Result<()> {
    run(async {
        let (repo, mut storage, workflow, node, workspace) = fixture().await?;
        fs::write(workspace.join("work.txt"), "selected\n")?;
        repo.jj_at(&workspace, &["describe", "-m", "selected"])?;
        let selected = workspace_commit(&repo, &workspace)?;
        fs::write(workspace.join("work.txt"), "current\n")?;
        let run = saved_run(&mut storage, &workflow, &node, "source-session").await?;
        let current = workspace_commit(&repo, &workspace)?;
        fs::write(workspace.join("work.txt"), "unsaved\n")?;
        let operation = repo.operation()?;
        let graph = storage.workflow_graph(workflow.id).await?;
        let history = storage.history_status(&repo.project).await?;
        let impact = storage
            .node_restore_impact(workflow.id, node.id, &selected)
            .await?;
        assert_eq!(impact.native_operation, operation);
        assert_eq!(impact.current.commit_id, current);
        assert_eq!(impact.selected.commit_id, selected);
        assert!(impact.parents_match);
        assert!(impact.recorded_tree_changed);
        assert!(
            impact.observed_blockers.is_empty(),
            "{:?}",
            impact.observed_blockers
        );
        assert_eq!(impact.changes.len(), 1);
        assert_eq!(impact.changes[0].node_ids, [node.id]);
        assert_eq!(impact.tasks.len(), 1);
        assert!(impact.tasks[0].native_affected);
        assert!(impact.tasks[0].allocation.as_ref().unwrap().ready);
        assert_eq!(impact.workspaces.len(), 1);
        assert_eq!(
            impact.workspaces[0].checkout,
            RestoreImpactCheckout::CurrentRecorded
        );
        assert_eq!(impact.sessions.len(), 1);
        assert_eq!(
            impact.sessions[0].session_id.as_deref(),
            Some("source-session")
        );
        assert_eq!(impact.sessions[0].run_ids, [run.id]);
        assert!(impact.sessions[0].source_node);
        let same = storage
            .node_restore_impact(workflow.id, node.id, &current)
            .await?;
        assert!(
            !same.recorded_tree_changed,
            "comparison must exclude unsaved files"
        );
        assert!(
            same.notes
                .iter()
                .any(|note| note.contains("do not prove a no-op"))
        );
        assert_eq!(repo.operation()?, operation);
        assert_eq!(fs::read_to_string(workspace.join("work.txt"))?, "unsaved\n");
        assert_eq!(workspace_commit(&repo, &workspace)?, current);
        assert_eq!(storage.run(run.id).await?, run);
        assert!(
            storage
                .node_restores(workflow.id, node.id)
                .await?
                .is_empty()
        );
        assert_eq!(storage.workflow_graph(workflow.id).await?, graph);
        assert_eq!(storage.history_status(&repo.project).await?, history);
        drop(storage);
        let storage = Storage::open(&repo.directory.path().join("restore.db")).await?;
        assert_eq!(
            storage
                .node_restore_impact(workflow.id, node.id, &selected)
                .await?,
            impact
        );
        assert_eq!(repo.operation()?, operation);
        Ok(())
    })
}

#[test]
fn impact_includes_transitive_merges_graph_only_tasks_and_unmanaged_workspaces()
-> anyhow::Result<()> {
    run(async {
        let (repo, mut storage, workflow, node, workspace) = fixture().await?;
        let selected = workspace_commit(&repo, &workspace)?;
        fs::write(workspace.join("api.txt"), "API\n")?;
        saved_run(&mut storage, &workflow, &node, "upstream").await?;
        let child = storage
            .create_node_from(workflow.id, "UI", &NodeParent::Node { node_id: node.id })
            .await?;
        storage
            .create_node_change_from_dependency(workflow.id, child.id)
            .await?;
        let child_workspace = repo.directory.path().join("child");
        storage
            .create_node_workspace(workflow.id, child.id, &child_workspace)
            .await?;
        saved_run(&mut storage, &workflow, &child, "downstream").await?;
        let sibling = storage
            .create_node_from(
                workflow.id,
                "Unrelated branch",
                &NodeParent::WorkflowBase {},
            )
            .await?;
        storage
            .create_node_change_from_dependency(workflow.id, sibling.id)
            .await?;
        let merge = storage
            .create_node_from_parents(
                workflow.id,
                "Merge",
                &[
                    NodeParent::Node { node_id: child.id },
                    NodeParent::Node {
                        node_id: sibling.id,
                    },
                ],
            )
            .await?;
        storage
            .create_node_change_from_dependency(workflow.id, merge.id)
            .await?;
        let prompt = storage
            .create_node_from(
                workflow.id,
                "Prompt only",
                &NodeParent::Node { node_id: merge.id },
            )
            .await?;
        let merge_commit = storage
            .review_node(workflow.id, merge.id)
            .await?
            .revision
            .commit_id;
        // Native descendant without a saved task, materialized in a foreign workspace.
        repo.jj(&["new", "--no-edit", &merge_commit, "-m", "unmanaged"])?;
        let external = recorded_value(
            &repo,
            &format!("({merge_commit}::) ~ {merge_commit}"),
            "commit_id",
        )?;
        let external_workspace = repo.directory.path().join("external");
        repo.jj(&[
            "workspace",
            "add",
            "--name",
            "external",
            "-r",
            &external,
            external_workspace.to_str().unwrap(),
        ])?;
        repo.jj_at(&external_workspace, &["edit", &external])?;
        let operation = repo.operation()?;
        let impact = storage
            .node_restore_impact(workflow.id, node.id, &selected)
            .await?;
        let task_ids: BTreeSet<_> = impact.tasks.iter().map(|task| task.node.id).collect();
        assert_eq!(
            task_ids,
            BTreeSet::from([node.id, child.id, merge.id, prompt.id])
        );
        assert!(
            !task_ids.contains(&sibling.id),
            "merge's unrelated input is not a descendant"
        );
        assert!(
            !impact
                .tasks
                .iter()
                .find(|t| t.node.id == prompt.id)
                .unwrap()
                .native_affected
        );
        assert_eq!(impact.changes.len(), 4);
        let merged = impact
            .changes
            .iter()
            .find(|c| c.node_ids == [merge.id])
            .unwrap();
        assert_eq!(merged.parent_commit_ids.len(), 2);
        assert!(
            impact
                .changes
                .iter()
                .any(|c| c.revision.commit_id == external && c.node_ids.is_empty())
        );
        assert!(impact.workspaces.iter().any(|w| w.name == "external"));
        assert!(
            impact
                .workspaces
                .iter()
                .any(|w| w.directory.as_ref() == Some(&child_workspace.canonicalize().unwrap()))
        );
        assert_eq!(impact.sessions.len(), 2);
        assert!(
            !impact
                .sessions
                .iter()
                .find(|s| s.node_id == child.id)
                .unwrap()
                .source_node
        );
        assert!(
            impact
                .observed_blockers
                .iter()
                .any(|b| b.contains("downstream restore"))
        );
        assert_eq!(repo.operation()?, operation);
        assert!(
            storage
                .restore_node_revision(workflow.id, node.id, &selected)
                .await
                .is_err(),
            "preview must not enable downstream application"
        );
        assert_eq!(repo.operation()?, operation);
        assert!(
            storage
                .node_restores(workflow.id, node.id)
                .await?
                .is_empty()
        );
        Ok(())
    })
}

#[test]
fn impact_refuses_invalid_foreign_missing_and_divergent_source_without_mutation()
-> anyhow::Result<()> {
    run(async {
        let (repo, storage, workflow, node, workspace) = fixture().await?;
        let selected = workspace_commit(&repo, &workspace)?;
        let foreign = recorded_value(&repo, "@", "commit_id")?;
        let operation = repo.operation()?;
        for selector in ["@", &selected[..12], &"f".repeat(40), &foreign] {
            assert!(
                storage
                    .node_restore_impact(workflow.id, node.id, selector)
                    .await
                    .is_err(),
                "{selector}"
            );
        }
        assert!(
            storage
                .node_restore_impact(workflow.id + 99, node.id, &selected)
                .await
                .is_err()
        );
        assert!(
            storage
                .node_restore_impact(workflow.id, node.id + 99, &selected)
                .await
                .is_err()
        );
        assert_eq!(repo.operation()?, operation);
        repo.jj_at(&workspace, &["describe", "-m", "newer"])?;
        repo.jj_at(
            &workspace,
            &[
                "--at-op",
                &operation,
                "--ignore-working-copy",
                "describe",
                "-r",
                &selected,
                "-m",
                "divergent",
            ],
        )?;
        // Merge operation heads intentionally as an external native operation.
        repo.jj(&["op", "log", "--limit", "1"])?;
        let operation = repo.operation()?;
        let error = storage
            .node_restore_impact(workflow.id, node.id, &selected)
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("missing or divergent"),
            "{error:#}"
        );
        assert_eq!(repo.operation()?, operation);
        assert!(
            storage
                .node_restores(workflow.id, node.id)
                .await?
                .is_empty()
        );
        Ok(())
    })
}

#[test]
fn impact_follows_native_ownership_into_other_workflows_and_keeps_graph_only_descendants()
-> anyhow::Result<()> {
    run(async {
        let (repo, mut storage, workflow, node, workspace) = fixture().await?;
        let selected = workspace_commit(&repo, &workspace)?;
        repo.jj(&["new", "--no-edit", &selected, "-m", "other workflow"])?;
        let descendant =
            recorded_value(&repo, &format!("({selected}::) ~ {selected}"), "change_id")?;
        let other = storage
            .create_workflow(&repo.project, "Other workflow")
            .await?;
        let owner = storage.create_node(other.id, "Native downstream").await?;
        storage
            .associate_node(other.id, owner.id, &descendant)
            .await?;
        let prompt = storage.create_node(other.id, "Later prompt").await?;
        storage.connect_nodes(other.id, owner.id, prompt.id).await?;
        let operation = repo.operation()?;
        let impact = storage
            .node_restore_impact(workflow.id, node.id, &selected)
            .await?;
        assert_eq!(impact.tasks.len(), 3);
        assert!(
            impact.tasks.iter().any(|t| t.node.id == owner.id
                && t.node.workflow_id == other.id
                && t.native_affected)
        );
        assert!(
            impact
                .tasks
                .iter()
                .any(|t| t.node.id == prompt.id && !t.native_affected)
        );
        assert!(impact.changes.iter().any(|c| c.node_ids == [owner.id]));
        assert_eq!(repo.operation()?, operation);
        Ok(())
    })
}

#[test]
fn impact_observes_stale_missing_and_shared_workspaces_without_repair() -> anyhow::Result<()> {
    run(async {
        let (repo, mut storage, workflow, node, workspace) = fixture().await?;
        let selected = workspace_commit(&repo, &workspace)?;
        let child = storage
            .create_node_from(workflow.id, "Child", &NodeParent::Node { node_id: node.id })
            .await?;
        storage
            .create_node_change_from_dependency(workflow.id, child.id)
            .await?;
        let child_workspace = repo.directory.path().join("child");
        storage
            .create_node_workspace(workflow.id, child.id, &child_workspace)
            .await?;
        let child_commit = workspace_commit(&repo, &child_workspace)?;
        let missing = storage
            .create_node_from(
                workflow.id,
                "Missing checkout",
                &NodeParent::Node { node_id: child.id },
            )
            .await?;
        storage
            .create_node_change_from_dependency(workflow.id, missing.id)
            .await?;
        let missing_workspace = repo.directory.path().join("missing");
        storage
            .create_node_workspace(workflow.id, missing.id, &missing_workspace)
            .await?;
        let missing_workspace = missing_workspace.canonicalize()?;
        let child_workspace = child_workspace.canonicalize()?;
        // The allocation remains, but its directory is no longer there.
        fs::rename(&missing_workspace, repo.directory.path().join("moved"))?;
        repo.jj(&[
            "--ignore-working-copy",
            "describe",
            "-r",
            &child_commit,
            "-m",
            "external rewrite",
        ])?;
        let extra = repo.directory.path().join("extra");
        repo.jj(&[
            "workspace",
            "add",
            "--name",
            "extra",
            "-r",
            &selected,
            extra.to_str().unwrap(),
        ])?;
        repo.jj_at(&extra, &["edit", &selected])?;
        fs::write(child_workspace.join("dirty.txt"), "keep this\n")?;
        let operation = repo.operation()?;
        let impact = storage
            .node_restore_impact(workflow.id, node.id, &selected)
            .await?;
        let stale = impact
            .workspaces
            .iter()
            .find(|w| w.directory.as_ref() == Some(&child_workspace))
            .unwrap();
        assert_eq!(
            stale.checkout,
            RestoreImpactCheckout::Stale {
                commit_id: child_commit
            }
        );
        let absent = impact
            .workspaces
            .iter()
            .find(|w| w.directory.as_ref() == Some(&missing_workspace))
            .unwrap();
        assert!(matches!(
            absent.checkout,
            RestoreImpactCheckout::Unknown { .. }
        ));
        assert!(
            impact
                .observed_blockers
                .iter()
                .any(|b| b.contains("another native workspace"))
        );
        assert_eq!(repo.operation()?, operation);
        assert_eq!(
            fs::read_to_string(child_workspace.join("dirty.txt"))?,
            "keep this\n"
        );
        assert!(!missing_workspace.exists());
        Ok(())
    })
}

#[test]
fn impact_returns_parent_policy_and_active_run_blockers_instead_of_hiding_the_report()
-> anyhow::Result<()> {
    run(async {
        let (repo, mut storage, workflow, node, workspace) = fixture().await?;
        let selected = workspace_commit(&repo, &workspace)?;
        // Rebase outside Waypoints: an old exact card is still inspectable but
        // cannot be restored across its changed exact parent set.
        let other = recorded_value(&repo, "@", "commit_id")?;
        repo.jj_at(&workspace, &["rebase", "-r", &selected, "-d", &other])?;
        let current = workspace_commit(&repo, &workspace)?;
        repo.jj(&["bookmark", "create", "protected", "-r", &current])?;
        repo.jj(&[
            "config",
            "set",
            "--repo",
            "revset-aliases.\"immutable_heads()\"",
            "protected",
        ])?;
        let operation = repo.operation()?;
        let impact = storage
            .node_restore_impact(workflow.id, node.id, &selected)
            .await?;
        assert!(!impact.parents_match);
        assert!(impact.changes.iter().any(|c| c.immutable));
        assert!(
            impact
                .observed_blockers
                .iter()
                .any(|b| b.contains("different exact parents"))
        );
        assert!(
            impact
                .observed_blockers
                .iter()
                .any(|b| b.contains("immutable"))
        );
        assert_eq!(repo.operation()?, operation);
        repo.jj(&[
            "config",
            "set",
            "--repo",
            "revset-aliases.\"immutable_heads()\"",
            "root()",
        ])?;
        let active = storage
            .create_run(workflow.id, node.id, Path::new("/bin/sh"))
            .await?;
        let operation = repo.operation()?;
        let impact = storage
            .node_restore_impact(workflow.id, node.id, &selected)
            .await?;
        assert_eq!(impact.active_run_ids, [active.id]);
        assert!(
            impact
                .observed_blockers
                .iter()
                .any(|b| b.contains("active/interrupted"))
        );
        assert_eq!(repo.operation()?, operation);
        assert_eq!(storage.run(active.id).await?, active);
        Ok(())
    })
}

#[test]
fn impact_reports_existing_merge_conflicts_without_claiming_to_predict_new_conflicts()
-> anyhow::Result<()> {
    run(async {
        let (repo, mut storage, workflow, node, workspace) = fixture().await?;
        let selected = workspace_commit(&repo, &workspace)?;
        fs::write(workspace.join("shared.txt"), "upstream\n")?;
        saved_run(&mut storage, &workflow, &node, "source").await?;
        fs::write(repo.project.join("shared.txt"), "other branch\n")?;
        repo.jj(&["describe", "-m", "other branch"])?;
        let other = storage
            .create_node_from(workflow.id, "Other", &NodeParent::WorkflowBase {})
            .await?;
        storage
            .associate_node(workflow.id, other.id, &repo.value("change_id")?)
            .await?;
        let merge = storage
            .create_node_from_parents(
                workflow.id,
                "Merge",
                &[
                    NodeParent::Node { node_id: node.id },
                    NodeParent::Node { node_id: other.id },
                ],
            )
            .await?;
        storage
            .create_node_change_from_dependency(workflow.id, merge.id)
            .await?;
        let operation = repo.operation()?;
        let impact = storage
            .node_restore_impact(workflow.id, node.id, &selected)
            .await?;
        let merged = impact
            .changes
            .iter()
            .find(|c| c.node_ids == [merge.id])
            .unwrap();
        assert!(merged.revision.conflicted);
        assert_eq!(merged.parent_commit_ids.len(), 2);
        assert!(
            impact
                .notes
                .iter()
                .any(|note| note.contains("future conflicts are unknown"))
        );
        assert_eq!(repo.operation()?, operation);
        Ok(())
    })
}

#[test]
fn impact_refuses_oversized_native_or_serialized_reports_without_partial_success()
-> anyhow::Result<()> {
    run(async {
        for native_limit in [true, false] {
            let (repo, storage, workflow, node, workspace) = fixture().await?;
            let description = "x".repeat(180_000);
            if native_limit {
                let parent = workspace_commit(&repo, &workspace)?;
                for index in 0..7 {
                    repo.jj(&[
                        "new",
                        "--no-edit",
                        &parent,
                        "-m",
                        &format!("{description}{index}"),
                    ])?;
                }
            } else {
                repo.jj_at(&workspace, &["describe", "-m", &description])?;
                let parent = workspace_commit(&repo, &workspace)?;
                for index in 0..2 {
                    let extra = repo.directory.path().join(format!("extra-{index}"));
                    repo.jj(&[
                        "workspace",
                        "add",
                        "--name",
                        &format!("extra-{index}"),
                        "-r",
                        &parent,
                        extra.to_str().unwrap(),
                    ])?;
                    repo.jj_at(&extra, &["edit", &parent])?;
                }
            }
            let selected = workspace_commit(&repo, &workspace)?;
            let operation = repo.operation()?;
            let error = storage
                .node_restore_impact(workflow.id, node.id, &selected)
                .await
                .unwrap_err();
            let detail = format!("{error:#}");
            assert!(
                detail.contains(if native_limit {
                    "exceeds 1 MiB"
                } else {
                    "response limit"
                }),
                "{detail}"
            );
            assert_eq!(repo.operation()?, operation);
            assert!(
                storage
                    .node_restores(workflow.id, node.id)
                    .await?
                    .is_empty()
            );
        }
        Ok(())
    })
}

#[test]
fn impact_needs_no_allocated_workspace_but_refuses_a_missing_source_change() -> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        let mut storage = Storage::open(&repo.directory.path().join("impact.db")).await?;
        let workflow = storage
            .create_workflow_from(
                &repo.project,
                "Preview before allocation",
                &BaseSource::Revision {
                    id: "root()".into(),
                },
            )
            .await?;
        let node = storage
            .create_node_from(workflow.id, "Task", &NodeParent::WorkflowBase {})
            .await?;
        storage
            .create_node_change_from_dependency(workflow.id, node.id)
            .await?;
        let selected = storage.review_node(workflow.id, node.id).await?.revision;
        let operation = repo.operation()?;
        let impact = storage
            .node_restore_impact(workflow.id, node.id, &selected.commit_id)
            .await?;
        assert!(impact.workspaces.is_empty());
        assert!(impact.tasks[0].allocation.is_none());
        assert!(
            impact
                .observed_blockers
                .iter()
                .any(|b| b.contains("dedicated workspace"))
        );
        assert_eq!(repo.operation()?, operation);
        repo.jj(&["abandon", &selected.commit_id])?;
        let operation = repo.operation()?;
        let error = storage
            .node_restore_impact(workflow.id, node.id, &selected.commit_id)
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("missing or divergent"),
            "{error:#}"
        );
        assert_eq!(repo.operation()?, operation);
        assert!(
            storage
                .node_restores(workflow.id, node.id)
                .await?
                .is_empty()
        );
        Ok(())
    })
}
