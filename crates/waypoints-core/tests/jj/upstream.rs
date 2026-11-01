use super::*;
use waypoints_core::run::{RunState, UpstreamStatus};

#[test]
fn inputs_are_informational_persistent_and_recaptured_for_continuation() -> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        let database = repo.directory.path().join("inputs.db");
        let mut storage = Storage::open(&database).await?;
        let workflow = storage
            .create_workflow_from(
                &repo.project,
                "Inputs",
                &BaseSource::Revision {
                    id: "root()".into(),
                },
            )
            .await?;
        let a = storage
            .create_node_from(workflow.id, "A", &NodeParent::WorkflowBase {})
            .await?;
        let upstream = storage
            .create_node_change_from_dependency(workflow.id, a.id)
            .await?
            .association
            .expect("native upstream")
            .change_id;
        repo.jj(&["edit", &upstream])?;
        fs::write(repo.project.join("upstream.txt"), "upstream work\n")?;
        repo.jj(&["new"])?;
        let b = storage
            .create_node_from(workflow.id, "B", &NodeParent::Node { node_id: a.id })
            .await?;
        storage
            .create_node_change_from_dependency(workflow.id, b.id)
            .await?;
        let workspace = repo.directory.path().join("b-workspace");
        storage
            .create_node_workspace(workflow.id, b.id, &workspace)
            .await?;
        let mut first = storage
            .create_run(workflow.id, b.id, Path::new("/bin/sh"))
            .await?;
        assert_eq!(first.prompt, "B");
        let saved = first.upstream_inputs.as_ref().expect("captured inputs");
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].parent, NodeParent::Node { node_id: a.id });
        assert_eq!(
            storage.run_upstream_status(first.id).await?,
            UpstreamStatus::Unchanged
        );
        first.state = RunState::Completed;
        first.session_id = Some("saved-session".into());
        storage.save_run(&first).await?;
        storage
            .update_node(workflow.id, a.id, "New upstream prompt")
            .await?;
        fs::write(workspace.join("own.txt"), "downstream work\n")?;
        repo.jj_at(&workspace, &["describe", "-m", "Downstream only"])?;
        assert_eq!(
            storage.run_upstream_status(first.id).await?,
            UpstreamStatus::Unchanged
        );
        repo.jj(&[
            "--ignore-working-copy",
            "describe",
            "-r",
            &upstream,
            "-m",
            "Upstream changed",
        ])?;
        let native = repo.operation()?;
        let history = storage.history_status(&repo.project).await?;
        fs::write(workspace.join("dirty.txt"), "leave this alone\n")?;
        assert_eq!(
            storage.run_upstream_status(first.id).await?,
            UpstreamStatus::Changed
        );
        assert_eq!(repo.operation()?, native);
        assert_eq!(storage.history_status(&repo.project).await?, history);
        assert_eq!(storage.run(first.id).await?, first);
        assert_eq!(
            fs::read_to_string(workspace.join("dirty.txt"))?,
            "leave this alone\n"
        );
        drop(storage);
        let mut storage = Storage::open(&database).await?;
        assert_eq!(
            storage.run_upstream_status(first.id).await?,
            UpstreamStatus::Changed
        );
        // Checkout repair remains explicit and separate from the informational hint.
        // Remove this test-owned probe before native repair snapshots the stale checkout.
        fs::remove_file(workspace.join("dirty.txt"))?;
        repo.jj_at(&workspace, &["workspace", "update-stale"])?;
        storage.verify_node_workspace(workflow.id, b.id).await?;
        let mut next = storage.continue_run(first.id, "Follow up").await?;
        assert_ne!(next.upstream_inputs, first.upstream_inputs);
        assert_eq!(next.prompt, "Follow up");
        assert_eq!(
            storage.run_upstream_status(next.id).await?,
            UpstreamStatus::Unchanged
        );
        assert_eq!(
            storage.run_upstream_status(first.id).await?,
            UpstreamStatus::Changed
        );
        next.state = RunState::Completed;
        storage.save_run(&next).await?;
        repo.jj(&["--ignore-working-copy", "abandon", &upstream])?;
        assert!(matches!(
            storage.run_upstream_status(first.id).await?,
            UpstreamStatus::Unknown { .. }
        ));
        assert_eq!(storage.run(first.id).await?, first);
        Ok(())
    })
}

#[test]
fn fixed_base_inputs_do_not_follow_external_rewrites() -> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        let base = repo.value("change_id")?;
        let mut storage = Storage::open(&repo.directory.path().join("inputs.db")).await?;
        let workflow = storage
            .create_workflow_from(
                &repo.project,
                "Inputs",
                &BaseSource::Revision { id: base.clone() },
            )
            .await?;
        let node = storage
            .create_node_from(workflow.id, "Root", &NodeParent::WorkflowBase {})
            .await?;
        storage
            .create_node_change_from_dependency(workflow.id, node.id)
            .await?;
        let workspace = repo.directory.path().join("root-workspace");
        storage
            .create_node_workspace(workflow.id, node.id, &workspace)
            .await?;
        let mut turn = storage
            .create_run(workflow.id, node.id, Path::new("/bin/sh"))
            .await?;
        assert_eq!(
            turn.upstream_inputs.as_ref().expect("base input")[0].commit_id,
            workflow
                .base
                .as_ref()
                .expect("fixed base")
                .revision
                .commit_id
        );
        turn.state = RunState::Completed;
        storage.save_run(&turn).await?;
        repo.jj(&[
            "--ignore-working-copy",
            "describe",
            "-r",
            &base,
            "-m",
            "Rewritten base change",
        ])?;
        assert_eq!(
            storage.run_upstream_status(turn.id).await?,
            UpstreamStatus::Unchanged
        );
        turn.upstream_inputs = None;
        storage.save_run(&turn).await?;
        assert!(matches!(
            storage.run_upstream_status(turn.id).await?,
            UpstreamStatus::Unknown { .. }
        ));
        Ok(())
    })
}
