use waypoints_core::storage::Storage;

fn run(test: impl std::future::Future<Output = anyhow::Result<()>>) -> anyhow::Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .build()?
        .block_on(test)
}

#[test]
fn rename_round_trips_verbatim_names_and_project_history_across_restarts() -> anyhow::Result<()> {
    run(async {
        let directory = tempfile::tempdir()?;
        let other_project = directory.path().join("other");
        std::fs::create_dir(&other_project)?;
        let database = directory.path().join("workflows.db");
        let mut storage = Storage::open(&database).await?;
        let workflow = storage
            .create_workflow(directory.path(), "Original")
            .await?;
        let sibling = storage.create_workflow(directory.path(), "Sibling").await?;
        let other = storage
            .create_workflow(&other_project, "Other project")
            .await?;
        let a = storage.create_node(workflow.id, "A").await?;
        let b = storage.create_node(workflow.id, "B").await?;
        storage.connect_nodes(workflow.id, a.id, b.id).await?;
        storage.create_node(sibling.id, "Sibling's edit").await?;
        storage
            .create_node(other.id, "Other project's edit")
            .await?;
        let original = storage.workflow_graph(workflow.id).await?;
        let other_history = storage.history_status(&other_project).await?;
        let previous = storage
            .history_status(directory.path())
            .await?
            .undo
            .expect("sibling edit");
        let name = "  Renamed 'workflow' 日本語\n\0 ";
        let renamed = storage.rename_workflow(workflow.id, name).await?;
        assert_eq!(renamed.id, workflow.id);
        assert_eq!(renamed.project_path, workflow.project_path);
        assert_eq!(renamed.name, name);
        assert_eq!(renamed.base, workflow.base);
        let mut expected = original.clone();
        expected.workflow = renamed.clone();
        assert_eq!(storage.workflow_graph(workflow.id).await?, expected);
        assert_eq!(
            storage.list_workflows(directory.path()).await?,
            vec![renamed.clone(), sibling.clone()]
        );
        let edit = storage
            .history_status(directory.path())
            .await?
            .undo
            .expect("rename edit");
        assert_eq!(edit.label, "Rename workflow");
        assert_eq!(edit.workflow_id, workflow.id);
        assert!(
            storage.undo(directory.path(), previous.id).await.is_err(),
            "stale selection must not undo a rename"
        );
        drop(storage);

        let mut storage = Storage::open(&database).await?;
        assert_eq!(storage.workflow(workflow.id).await?, renamed);
        let undone = storage.undo(directory.path(), edit.id).await?;
        assert_eq!(undone.undo, Some(previous));
        assert_eq!(undone.redo, Some(edit.clone()));
        assert_eq!(storage.workflow_graph(workflow.id).await?, original);
        drop(storage);
        let mut storage = Storage::open(&database).await?;
        storage.redo(directory.path(), edit.id).await?;
        assert_eq!(storage.workflow_graph(workflow.id).await?, expected);
        assert_eq!(storage.workflow(sibling.id).await?, sibling);
        assert_eq!(storage.history_status(&other_project).await?, other_history);
        Ok(())
    })
}

#[test]
fn rejected_and_identical_renames_preserve_redo_but_a_new_rename_discards_it() -> anyhow::Result<()>
{
    run(async {
        let directory = tempfile::tempdir()?;
        let mut storage = Storage::open(&directory.path().join("workflows.db")).await?;
        let workflow = storage
            .create_workflow(directory.path(), "Original")
            .await?;
        let duplicate = storage
            .create_workflow(directory.path(), "Duplicate")
            .await?;
        storage.rename_workflow(workflow.id, "First rename").await?;
        let edit = storage
            .history_status(directory.path())
            .await?
            .undo
            .expect("rename");
        let pending = storage.undo(directory.path(), edit.id).await?;
        for name in ["", " \n\t", "\u{2003}"] {
            assert!(
                storage.rename_workflow(workflow.id, name).await.is_err(),
                "blank name {name:?}"
            );
        }
        assert!(storage.rename_workflow(i64::MAX, "Missing").await.is_err());
        assert_eq!(
            storage.rename_workflow(workflow.id, "Original").await?,
            workflow
        );
        assert_eq!(storage.history_status(directory.path()).await?, pending);
        let renamed = storage
            .rename_workflow(workflow.id, &duplicate.name)
            .await?;
        assert_eq!(renamed.name, duplicate.name, "duplicate names are allowed");
        let replacement = storage
            .history_status(directory.path())
            .await?
            .undo
            .expect("replacement");
        assert_ne!(replacement.id, edit.id);
        assert!(
            storage
                .history_status(directory.path())
                .await?
                .redo
                .is_none()
        );
        storage.undo(directory.path(), replacement.id).await?;
        assert!(
            storage.redo(directory.path(), edit.id).await.is_err(),
            "old redo selection must not replay a replacement"
        );
        storage.redo(directory.path(), replacement.id).await?;
        assert_eq!(storage.workflow(workflow.id).await?, renamed);
        Ok(())
    })
}
