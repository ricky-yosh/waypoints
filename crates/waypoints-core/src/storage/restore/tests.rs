use super::*;

#[test]
fn failed_reservation_rolls_back_journal_identity_and_session_barrier() -> anyhow::Result<()> {
    tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(async {
        let directory = tempfile::tempdir()?;
        let mut storage = Storage::open(&directory.path().join("restore.db")).await?;
        let workflow = storage.create_workflow(directory.path(), "Journal").await?;
        let node = storage.create_node(workflow.id, "Task").await?;
        let revision = Revision {
            change_id: "k".repeat(32), commit_id: "0".repeat(40),
            description: String::new(), conflicted: false,
        };
        let run = Run {
            id: 1, workflow_id: workflow.id, node_id: node.id,
            prompt: node.prompt, directory: directory.path().join("workspace"),
            change_id: revision.change_id.clone(), executable: "/bin/sh".into(),
            session_id: Some("session".into()), continued_from: None, process_id: None,
            state: crate::run::RunState::Completed,
            snapshot: SnapshotState::Saved { revision: revision.clone() },
            upstream_inputs: None,
        };
        let connection = storage.database.connect()?;
        connection.execute(
            "INSERT INTO agent_runs(id, node_id, directory, active, record) VALUES (1, ?1, ?2, 0, ?3)",
            turso::params![node.id, run.directory.to_str(), serde_json::to_string(&run)?],
        ).await?;
        let mut operation = RestoreOperation {
            id: 0, run_id: run.id, workflow_id: workflow.id, node_id: node.id,
            workspace: WorkspaceAssociation {
                repository_path: directory.path().to_owned(),
                directory: run.directory.clone(), change_id: revision.change_id.clone(),
                name: "test".into(),
            },
            source: revision.clone(), before: revision, preserved: None, after: None,
            native_operation: "1".repeat(128), session_barrier: 0,
            state: RestoreState::Unresolved,
        };
        storage.reserve_restore(&mut operation).await?;
        operation.state = RestoreState::Completed;
        storage.save_restore(&operation).await?;
        let saved = storage.run_restores(run.id).await?;
        // The initial id:0 reservation passes; its second write (real ID) fails.
        // This exercises rollback after INSERT and last_insert_rowid, not just
        // validation before a transaction starts.
        connection.execute(
            r#"CREATE UNIQUE INDEX fail_restore ON restore_operations(run_id)
               WHERE record NOT LIKE '%"id":0,%'"#, (),
        ).await?;
        let mut failed = operation.clone();
        failed.id = 0;
        failed.state = RestoreState::Unresolved;
        assert!(storage.reserve_restore(&mut failed).await.is_err());
        assert_eq!(storage.run_restores(run.id).await?, saved);
        storage.ensure_restores_clear().await?;
        connection.execute("DROP INDEX fail_restore", ()).await?;
        failed.id = 0;
        storage.reserve_restore(&mut failed).await?;
        assert_eq!(failed.id, operation.id + 1, "rolled-back IDs should not consume a slot");
        assert!(storage.ensure_restores_clear().await.is_err());
        assert!(storage.ensure_node_idle(node.id).await.is_err());
        drop(storage);
        let storage = Storage::open(&directory.path().join("restore.db")).await?;
        assert!(storage.ensure_restores_clear().await.is_err());
        assert_eq!(storage.restore_operation(failed.id).await?, failed);
        Ok(())
    })
}
