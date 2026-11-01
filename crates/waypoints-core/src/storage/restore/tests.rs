use super::*;

#[test]
fn failed_downstream_completion_keeps_checkpoints_global_guard_and_child_session_barrier()
-> anyhow::Result<()> {
    tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(async {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("restore.db");
        let mut storage = Storage::open(&path).await?;
        let workflow = storage.create_workflow(directory.path(), "Journal failure").await?;
        let source = storage.create_node(workflow.id, "Source").await?;
        let child = storage.create_node(workflow.id, "Child").await?;
        let revision = Revision { change_id: "k".repeat(32), commit_id: "1".repeat(40),
            description: String::new(), conflicted: false };
        let run = Run {
            id: 7, workflow_id: workflow.id, node_id: child.id, prompt: child.prompt,
            directory: directory.path().join("child"), change_id: revision.change_id.clone(),
            executable: "/bin/sh".into(), session_id: Some("old-child-session".into()),
            continued_from: None, process_id: None, state: crate::run::RunState::Completed,
            snapshot: SnapshotState::NotRecorded, upstream_inputs: None,
        };
        let connection = storage.database.connect()?;
        connection.execute(
            "INSERT INTO agent_runs(id, node_id, directory, active, record) VALUES (7, ?1, ?2, 0, ?3)",
            turso::params![child.id, run.directory.to_str(), serde_json::to_string(&run)?],
        ).await?;
        let mut operation = RestoreOperation {
            id: 0, run_id: None, workflow_id: workflow.id, node_id: source.id,
            workspace: WorkspaceAssociation {
                repository_path: directory.path().to_owned(), directory: directory.path().join("source"),
                change_id: revision.change_id.clone(), name: "source".into(),
            },
            source: revision.clone(), before: revision.clone(), preserved: Some(revision.clone()),
            after: None, native_operation: "2".repeat(128), session_barrier: 0,
            state: RestoreState::Unresolved,
            downstream: Some(RestoreDownstream {
                preview_restore_watermark: 1,
                graph: storage.workflow_graph(workflow.id).await?,
                nodes: vec![RestoreAffectedNode {
                    node_id: child.id, before: Some(revision.clone()), before_parent_commit_ids: vec![],
                    workspace: None, preserved: Some(revision.clone()), after: None,
                    needs_workspace_update: false,
                }],
                context_notice: "not rerun".into(),
            }),
        };
        let mut old = operation.clone();
        old.id = 1;
        old.state = RestoreState::Completed;
        old.downstream = None;
        connection.execute(
            "INSERT INTO restore_operations(id, run_id, node_id, active, session_barrier, record)
             VALUES (1, NULL, ?1, 0, 0, ?2)",
            turso::params![source.id, serde_json::to_string(&old)?],
        ).await?;
        storage.reserve_restore(&mut operation).await?;
        let saved = storage.restore_operation(operation.id).await?;
        connection.execute(
            "CREATE UNIQUE INDEX fail_completed_restore ON restore_operations(active)
             WHERE json_extract(record, '$.state') = 'completed'", (),
        ).await?;
        operation.after = Some(revision);
        operation.state = RestoreState::Completed;
        assert!(storage.save_restore(&operation).await.is_err());
        assert_eq!(storage.restore_operation(operation.id).await?, saved);
        assert!(storage.ensure_restores_clear().await.is_err());
        assert!(storage.ensure_session_after_restore(&run).await.is_err());
        drop(connection);
        drop(storage);
        let storage = Storage::open(&path).await?;
        assert_eq!(storage.node_restores(workflow.id, child.id).await?, std::slice::from_ref(&saved));
        assert!(storage.ensure_restores_clear().await.is_err());
        assert!(storage.ensure_session_after_restore(&run).await.is_err());
        storage.database.connect()?.execute("DROP INDEX fail_completed_restore", ()).await?;
        operation.state = RestoreState::Recovered;
        storage.save_restore(&operation).await?;
        storage.ensure_restores_clear().await?;
        assert!(storage.ensure_session_after_restore(&run).await.is_err(),
            "recovery must not re-enable old child memory");
        Ok(())
    })
}

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
            id: 0, run_id: Some(run.id), workflow_id: workflow.id, node_id: node.id,
            workspace: WorkspaceAssociation {
                repository_path: directory.path().to_owned(),
                directory: run.directory.clone(), change_id: revision.change_id.clone(),
                name: "test".into(),
            },
            source: revision.clone(), before: revision, preserved: None, after: None,
            native_operation: "1".repeat(128), session_barrier: 0,
            state: RestoreState::Unresolved,
            downstream: None,
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
        // Reopen actual serialized records through the legacy run-only schema,
        // including its unresolved reservation and continuation barrier.
        connection.execute(
            "CREATE TABLE restore_operations_legacy (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                run_id INTEGER NOT NULL REFERENCES agent_runs(id),
                node_id INTEGER NOT NULL REFERENCES nodes(id),
                active INTEGER NOT NULL CHECK(active IN (0, 1)),
                session_barrier INTEGER NOT NULL,
                record TEXT NOT NULL
            )", (),
        ).await?;
        connection.execute("INSERT INTO restore_operations_legacy SELECT * FROM restore_operations", ()).await?;
        connection.execute("DROP TABLE restore_operations", ()).await?;
        connection.execute("ALTER TABLE restore_operations_legacy RENAME TO restore_operations", ()).await?;
        connection.execute(
            "CREATE UNIQUE INDEX one_restore_operation ON restore_operations(active) WHERE active = 1", (),
        ).await?;
        drop(storage);
        let storage = Storage::open(&directory.path().join("restore.db")).await?;
        assert!(storage.ensure_restores_clear().await.is_err());
        assert_eq!(storage.restore_operation(failed.id).await?, failed);
        assert_eq!(storage.restore_operation(operation.id).await?, operation);
        assert!(storage.ensure_session_after_restore(&run).await.is_err());
        Ok(())
    })
}
