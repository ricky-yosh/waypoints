use std::fs;
use std::future::Future;

use waypoints_core::storage::{Storage, Workflow, WorkflowFilter};

fn run(test: impl Future<Output = anyhow::Result<()>>) -> anyhow::Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .build()?
        .block_on(test)
}

#[test]
fn archive_preserves_redo_and_name_replay_preserves_archive() -> anyhow::Result<()> {
    run(async {
        let directory = tempfile::tempdir()?;
        let mut storage = Storage::open(&directory.path().join("workflows.db")).await?;
        let workflow = storage
            .create_workflow(directory.path(), "Original")
            .await?;
        storage.rename_workflow(workflow.id, "Renamed").await?;
        let history = storage.history_status(directory.path()).await?;
        let edit = history.undo.expect("rename must be undoable").id;
        storage.undo(directory.path(), edit).await?;
        let pending = storage.history_status(directory.path()).await?;
        storage.archive_workflow(workflow.id).await?;
        assert_eq!(storage.history_status(directory.path()).await?, pending);
        storage.redo(directory.path(), edit).await?;
        let archived = storage.workflow(workflow.id).await?;
        assert!(archived.archived);
        assert_eq!(archived.name, "Renamed");
        assert!(
            storage
                .rename_workflow(workflow.id, "Still archived")
                .await?
                .archived
        );
        storage.unarchive_workflow(workflow.id).await?;
        assert_eq!(storage.workflow(workflow.id).await?.name, "Still archived");
        Ok(())
    })
}

#[test]
fn archive_round_trip_preserves_graph_history_and_project_scoping() -> anyhow::Result<()> {
    run(async {
        let directory = tempfile::tempdir()?;
        let database = directory.path().join("workflows.db");
        let other_project = directory.path().join("other");
        fs::create_dir(&other_project)?;
        let mut storage = Storage::open(&database).await?;
        let original = storage.create_workflow(directory.path(), "First").await?;
        let second = storage.create_workflow(directory.path(), "Second").await?;
        let other = storage.create_workflow(&other_project, "Other").await?;
        let node = storage.create_node(original.id, "Keep this prompt").await?;
        let before = storage.workflow_graph(original.id).await?;
        let history = storage.history_status(directory.path()).await?;
        let archived = storage.archive_workflow(original.id).await?;
        let mut expected = original.clone();
        expected.archived = true;
        assert_eq!(archived, expected);
        assert_eq!(storage.archive_workflow(original.id).await?, archived);
        assert_eq!(storage.workflow(original.id).await?, archived);
        assert_eq!(storage.node(original.id, node.id).await?, node);
        assert_eq!(
            storage.list_workflows(directory.path()).await?,
            vec![second.clone()]
        );
        assert_eq!(
            storage
                .list_workflows_filtered(directory.path(), WorkflowFilter::Archived)
                .await?,
            vec![archived.clone()]
        );
        assert_eq!(
            storage
                .list_workflows_filtered(directory.path(), WorkflowFilter::All)
                .await?,
            vec![archived.clone(), second]
        );
        assert_eq!(storage.list_workflows(&other_project).await?, vec![other]);
        assert!(
            storage
                .list_workflows_filtered(&other_project, WorkflowFilter::Archived)
                .await?
                .is_empty()
        );
        assert_eq!(storage.history_status(directory.path()).await?, history);
        let mut archived_graph = before.clone();
        archived_graph.workflow = archived.clone();
        assert_eq!(storage.workflow_graph(original.id).await?, archived_graph);
        assert!(storage.archive_workflow(i64::MAX).await.is_err());
        assert!(storage.unarchive_workflow(i64::MAX).await.is_err());
        drop(storage);

        let mut storage = Storage::open(&database).await?;
        assert_eq!(storage.workflow(original.id).await?, archived);
        assert_eq!(storage.unarchive_workflow(original.id).await?, original);
        assert_eq!(storage.unarchive_workflow(original.id).await?, original);
        assert_eq!(storage.workflow_graph(original.id).await?, before);
        assert_eq!(storage.history_status(directory.path()).await?, history);
        drop(storage);
        let storage = Storage::open(&database).await?;
        assert!(!storage.workflow(original.id).await?.archived);
        Ok(())
    })
}

#[test]
fn pre_archive_databases_and_records_default_to_active() -> anyhow::Result<()> {
    run(async {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("legacy.db");
        // The original workflow schema, before any archive metadata existed.
        let database = turso::Builder::new_local(path.to_str().expect("database path"))
            .build()
            .await?;
        let connection = database.connect()?;
        connection.execute(
            "CREATE TABLE workflows (id INTEGER PRIMARY KEY, project_path TEXT NOT NULL, name TEXT NOT NULL)",
            (),
        ).await?;
        let project = fs::canonicalize(directory.path())?;
        connection
            .execute(
                "INSERT INTO workflows VALUES (7, ?1, 'Existing')",
                [project.to_str().expect("project path")],
            )
            .await?;
        drop(connection);
        drop(database);

        let mut storage = Storage::open(&path).await?;
        let saved = storage.workflow(7).await?;
        assert!(!saved.archived);
        assert_eq!(storage.list_workflows(&project).await?, vec![saved.clone()]);
        let legacy: Workflow = serde_json::from_value(serde_json::json!({
            "id":7, "project_path":project, "name":"Existing", "base":null
        }))?;
        assert_eq!(legacy, saved);
        storage.archive_workflow(7).await?;
        drop(storage);
        let storage = Storage::open(&path).await?;
        assert!(storage.workflow(7).await?.archived);
        Ok(())
    })
}
