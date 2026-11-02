use super::*;
use waypoints_core::workspace::WorkspaceUpdateOutcome;

#[test]
fn clean_stale_update_is_verified_and_failed_metadata_acknowledgement_never_replays()
-> anyhow::Result<()> {
    run(async {
        for fail_acknowledgement in [false, true] {
            let (repo, mut storage, workflow, nodes) = reparent_fixture(false).await?;
            let node = &nodes[1];
            let path = repo.directory.path().join("dedicated");
            storage
                .create_node_workspace(workflow.id, node.id, &path)
                .await?;
            storage
                .reparent_node(
                    workflow.id,
                    node.id,
                    &NodeParent::WorkflowBase {},
                    Some(UndoBoundaryAcknowledgement::AcceptUndoBoundary),
                )
                .await?;
            let history = storage.history_status(&repo.project).await?;
            let database_path = repo.directory.path().join("workflow.db");
            let database = turso::Builder::new_local(database_path.to_str().expect("path"))
                .build()
                .await?;
            let connection = database.connect()?;
            if fail_acknowledgement {
                connection.execute(
                    "CREATE TRIGGER reject_update_ack BEFORE UPDATE OF ready ON node_workspaces
                     WHEN NEW.ready = 1 BEGIN SELECT RAISE(FAIL, 'injected acknowledgement failure'); END",
                    (),
                ).await?;
            }
            let result = storage
                .update_stale_node_workspace(workflow.id, node.id)
                .await;
            assert_eq!(storage.history_status(&repo.project).await?, history);
            if fail_acknowledgement {
                assert!(result.is_err());
                assert!(matches!(
                    storage.node_workspace(workflow.id, node.id).await?.state,
                    WorkspaceState::Incomplete
                ));
                let head = repo.operation()?;
                drop(storage);
                let mut storage = Storage::open(&database_path).await?;
                assert!(
                    storage
                        .update_stale_node_workspace(workflow.id, node.id)
                        .await
                        .is_err()
                );
                assert_eq!(repo.operation()?, head);
                connection
                    .execute("DROP TRIGGER reject_update_ack", ())
                    .await?;
                // Native files are already fresh; only explicit verification acknowledges them.
                storage.verify_node_workspace(workflow.id, node.id).await?;
                assert_eq!(repo.operation()?, head);
            } else {
                assert_eq!(result?.outcome, WorkspaceUpdateOutcome::Updated);
                storage.verify_node_workspace(workflow.id, node.id).await?;
                let head = repo.operation()?;
                assert_eq!(
                    storage
                        .update_stale_node_workspace(workflow.id, node.id)
                        .await?
                        .outcome,
                    WorkspaceUpdateOutcome::AlreadyFresh
                );
                assert_eq!(repo.operation()?, head);
                drop(storage);
                let storage = Storage::open(&database_path).await?;
                assert!(matches!(
                    storage.node_workspace(workflow.id, node.id).await?.state,
                    WorkspaceState::Ready { .. }
                ));
            }
        }
        Ok(())
    })
}

#[test]
fn workspace_update_fresh_is_readonly_and_rebase_update_survives_restart() -> anyhow::Result<()> {
    run(async {
        let (repo, mut storage, workflow, nodes) = reparent_fixture(false).await?;
        let node = &nodes[1];
        let path = repo.directory.path().join("dedicated");
        storage
            .create_node_workspace(workflow.id, node.id, &path)
            .await?;
        fs::write(path.join("authentication.txt"), "dirty tracked edit\n")?;
        fs::write(path.join("untracked.txt"), "untracked edit\n")?;
        fs::write(path.join(".gitignore"), "ignored.txt\nuntracked.txt\n")?;
        fs::write(path.join("ignored.txt"), "ignored edit\n")?;
        let operation = repo.operation()?;
        let history = storage.history_status(&repo.project).await?;
        assert_eq!(
            storage
                .update_stale_node_workspace(workflow.id, node.id)
                .await?
                .outcome,
            WorkspaceUpdateOutcome::AlreadyFresh
        );
        assert_eq!(
            repo.operation()?,
            operation,
            "fresh no-op must not snapshot dirty files"
        );
        storage
            .reparent_node(
                workflow.id,
                node.id,
                &NodeParent::WorkflowBase {},
                Some(UndoBoundaryAcknowledgement::AcceptUndoBoundary),
            )
            .await?;
        assert!(
            storage
                .verify_node_workspace(workflow.id, node.id)
                .await
                .is_err()
        );
        let result = storage
            .update_stale_node_workspace(workflow.id, node.id)
            .await;
        // jj 0.46 snapshots dirty stale files on their old operation. Refusing
        // the resulting divergent heads must not proceed to overwrite checkout files.
        assert!(result.is_err());
        assert_eq!(
            fs::read_to_string(path.join("authentication.txt"))?,
            "dirty tracked edit\n"
        );
        assert_eq!(
            fs::read_to_string(path.join("untracked.txt"))?,
            "untracked edit\n"
        );
        assert_eq!(
            fs::read_to_string(path.join("ignored.txt"))?,
            "ignored edit\n"
        );
        let state = storage.node_workspace(workflow.id, node.id).await?;
        assert!(matches!(state.state, WorkspaceState::Incomplete));
        assert_ne!(
            storage.history_status(&repo.project).await?,
            history,
            "reparent creates its own undo boundary"
        );
        drop(storage);
        let mut storage = Storage::open(&repo.directory.path().join("workflow.db")).await?;
        assert_eq!(storage.node_workspace(workflow.id, node.id).await?, state);
        if result.is_err() {
            // Readonly @ deliberately refuses divergent heads; compare that
            // observation across replay refusal without merging them for the test.
            let operation = repo
                .operation()
                .expect_err("dirty stale update leaves divergent heads")
                .to_string();
            assert!(
                storage
                    .update_stale_node_workspace(workflow.id, node.id)
                    .await
                    .is_err()
            );
            assert_eq!(
                repo.operation()
                    .expect_err("replay must leave heads unchanged")
                    .to_string(),
                operation,
                "uncertain native outcome must not replay"
            );
        }
        Ok(())
    })
}

#[test]
fn workspace_update_refuses_missing_switched_shared_and_busy_allocations() -> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        let mut storage = Storage::open(&repo.directory.path().join("update.db")).await?;
        let workflow = storage.create_workflow(&repo.project, "Update").await?;
        let node = storage.create_node(workflow.id, "Node").await?;
        assert!(
            storage
                .update_stale_node_workspace(workflow.id, node.id)
                .await
                .is_err()
        );
        storage
            .create_node_change(workflow.id, node.id, "root()")
            .await?;
        let path = repo.directory.path().join("dedicated");
        let workspace = storage
            .create_node_workspace(workflow.id, node.id, &path)
            .await?
            .workspace
            .unwrap();
        assert!(
            storage
                .update_stale_node_workspace(workflow.id + 1, node.id)
                .await
                .is_err()
        );
        let mut reservation = storage
            .create_run(workflow.id, node.id, Path::new("/bin/sh"))
            .await?;
        let operation = repo.operation()?;
        assert!(
            storage
                .update_stale_node_workspace(workflow.id, node.id)
                .await
                .is_err()
        );
        reservation.state = waypoints_core::run::RunState::Interrupted {
            detail: "test writer stopped unexpectedly".to_owned(),
            phase: waypoints_core::run::InterruptionPhase::Execution,
        };
        storage.save_run(&reservation).await?;
        assert!(
            storage
                .update_stale_node_workspace(workflow.id, node.id)
                .await
                .is_err()
        );
        reservation.state = waypoints_core::run::RunState::Completed;
        storage.save_run(&reservation).await?;
        assert_eq!(repo.operation()?, operation);
        let other = storage.create_node(workflow.id, "Shared").await?;
        storage
            .associate_node(workflow.id, other.id, &workspace.change_id)
            .await?;
        assert!(
            storage
                .update_stale_node_workspace(workflow.id, node.id)
                .await
                .is_err()
        );
        let original = repo.value("change_id")?;
        storage
            .associate_node(workflow.id, other.id, &original)
            .await?;
        // A separate native workspace editing the node is refused too.
        let extra = repo.directory.path().join("extra");
        repo.jj(&["workspace", "add", extra.to_str().unwrap()])?;
        repo.jj_at(&extra, &["edit", &workspace.change_id])?;
        assert!(
            storage
                .update_stale_node_workspace(workflow.id, node.id)
                .await
                .is_err()
        );
        repo.jj(&["--ignore-working-copy", "workspace", "forget", "extra"])?;
        repo.jj_at(&path, &["new"])?;
        assert!(
            storage
                .update_stale_node_workspace(workflow.id, node.id)
                .await
                .is_err()
        );
        repo.jj_at(&path, &["edit", &workspace.change_id])?;
        let moved = repo.directory.path().join("moved");
        fs::rename(&path, &moved)?;
        let operation = repo.operation()?;
        assert!(
            storage
                .update_stale_node_workspace(workflow.id, node.id)
                .await
                .is_err()
        );
        assert!(!path.exists());
        assert_eq!(repo.operation()?, operation);
        Ok(())
    })
}
