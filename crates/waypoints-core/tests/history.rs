use std::fs;
use std::future::Future;

use waypoints_core::storage::Storage;

#[test]
fn existing_nodes_form_a_baseline_without_invented_history() -> anyhow::Result<()> {
    run(async {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("legacy.db");
        let project = fs::canonicalize(directory.path())?;
        {
            let database = turso::Builder::new_local(path.to_str().expect("database path"))
                .build()
                .await?;
            let connection = database.connect()?;
            connection.execute(
                "CREATE TABLE workflows (id INTEGER PRIMARY KEY, project_path TEXT NOT NULL, name TEXT NOT NULL)", (),
            ).await?;
            connection.execute(
                "CREATE TABLE nodes (id INTEGER PRIMARY KEY, workflow_id INTEGER NOT NULL, prompt TEXT NOT NULL, UNIQUE(workflow_id,id))", (),
            ).await?;
            connection
                .execute(
                    "INSERT INTO workflows VALUES (1, ?1, 'Existing workflow')",
                    [project.to_str()],
                )
                .await?;
            connection
                .execute(
                    "INSERT INTO nodes VALUES (42, 1, 'Existing assignment')",
                    (),
                )
                .await?;
        }
        let mut storage = Storage::open(&path).await?;
        let baseline = storage.workflow_graph(1).await?;
        let status = storage.history_status(&project).await?;
        assert!(status.undo.is_none());
        assert!(status.redo.is_none());
        storage.update_node(1, 42, "New assignment").await?;
        let edit = storage
            .history_status(&project)
            .await?
            .undo
            .expect("new prompt edit");
        storage.undo(&project, edit.id).await?;
        assert_eq!(storage.workflow_graph(1).await?, baseline);
        assert!(storage.history_status(&project).await?.undo.is_none());
        let new = storage.create_node(1, "New node").await?;
        assert!(new.id > 42, "migrated identities must stay reserved");
        Ok(())
    })
}

fn run(test: impl Future<Output = anyhow::Result<()>>) -> anyhow::Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .build()?
        .block_on(test)
}

#[test]
fn history_round_trips_values_identities_and_cursor_across_restarts() -> anyhow::Result<()> {
    run(async {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("workflows.db");
        let project = directory.path();
        let mut storage = Storage::open(&path).await?;
        let workflow = storage.create_workflow(project, "History").await?;
        assert!(storage.history_status(project).await?.undo.is_none());
        assert!(storage.undo(project, 1).await.is_err());
        let baseline = storage.workflow_graph(workflow.id).await?;
        let first = storage
            .create_node(workflow.id, "  Original\n'prompt'\0 ")
            .await?;
        let second = storage.create_node(workflow.id, "Second").await?;
        storage
            .connect_nodes(workflow.id, first.id, second.id)
            .await?;
        storage
            .update_node(workflow.id, first.id, "Revised\nassignment")
            .await?;
        let final_graph = storage.workflow_graph(workflow.id).await?;
        let mut actions = Vec::new();
        for label in ["Edit prompt", "Connect nodes", "Create node", "Create node"] {
            let status = storage.history_status(project).await?;
            let action = status.undo.expect("pending undo");
            assert_eq!(action.label, label);
            let next = storage.undo(project, action.id).await?;
            assert_eq!(next.redo.as_ref(), Some(&action));
            actions.push(action);
        }
        assert_eq!(storage.workflow_graph(workflow.id).await?, baseline);
        drop(storage);
        let mut storage = Storage::open(&path).await?;
        for action in actions.iter().rev() {
            assert_eq!(
                storage.history_status(project).await?.redo.as_ref(),
                Some(action)
            );
            storage.redo(project, action.id).await?;
        }
        assert_eq!(storage.workflow_graph(workflow.id).await?, final_graph);
        assert!(storage.history_status(project).await?.redo.is_none());
        assert!(storage.redo(project, actions[0].id).await.is_err());
        drop(storage);
        let storage = Storage::open(&path).await?;
        assert_eq!(
            storage.history_status(project).await?.undo.as_ref(),
            Some(&actions[0])
        );
        assert_eq!(storage.workflow_graph(workflow.id).await?, final_graph);
        Ok(())
    })
}

#[test]
fn history_is_project_chronological_not_workflow_local_and_ids_are_not_reused() -> anyhow::Result<()>
{
    run(async {
        let directory = tempfile::tempdir()?;
        let other_project = directory.path().join("other");
        fs::create_dir(&other_project)?;
        let mut storage = Storage::open(&directory.path().join("workflows.db")).await?;
        let first = storage.create_workflow(directory.path(), "First").await?;
        let second = storage.create_workflow(directory.path(), "Second").await?;
        let other = storage.create_workflow(&other_project, "Other").await?;
        let a = storage.create_node(first.id, "A").await?;
        let b = storage.create_node(second.id, "B").await?;
        let latest = storage
            .history_status(directory.path())
            .await?
            .undo
            .expect("B edit");
        assert_eq!(latest.workflow_id, second.id);
        storage.undo(directory.path(), latest.id).await?;
        let c = storage.create_node(other.id, "C").await?;
        assert_ne!(c.id, b.id, "another project must not steal a redo identity");
        storage.redo(directory.path(), latest.id).await?;
        assert_eq!(storage.node(second.id, b.id).await?, b);
        assert_eq!(storage.node(first.id, a.id).await?, a);
        assert_eq!(storage.node(other.id, c.id).await?, c);
        assert_eq!(
            storage
                .history_status(&other_project)
                .await?
                .undo
                .expect("C edit")
                .workflow_id,
            other.id
        );
        Ok(())
    })
}

#[test]
fn rejected_and_noop_edits_preserve_redo_new_edits_drop_only_own_project_tail() -> anyhow::Result<()>
{
    run(async {
        let directory = tempfile::tempdir()?;
        let other_project = directory.path().join("other");
        fs::create_dir(&other_project)?;
        let mut storage = Storage::open(&directory.path().join("workflows.db")).await?;
        let workflow = storage.create_workflow(directory.path(), "Graph").await?;
        let other = storage.create_workflow(&other_project, "Other").await?;
        let a = storage.create_node(workflow.id, "A").await?;
        let b = storage.create_node(workflow.id, "B").await?;
        storage.connect_nodes(workflow.id, a.id, b.id).await?;
        storage.create_node(other.id, "Other node").await?;
        let other_status = storage.history_status(&other_project).await?;
        storage
            .update_node(workflow.id, a.id, "Before undo")
            .await?;
        let action = storage
            .history_status(directory.path())
            .await?
            .undo
            .expect("prompt edit");
        let pending = storage.undo(directory.path(), action.id).await?;
        assert!(storage.create_node(workflow.id, " \t").await.is_err());
        assert!(storage.create_node(9999, "missing workflow").await.is_err());
        assert!(storage.update_node(workflow.id, b.id, "").await.is_err());
        assert!(
            storage
                .update_node(other.id, a.id, "wrong workflow")
                .await
                .is_err()
        );
        assert!(
            storage
                .connect_nodes(workflow.id, a.id, a.id)
                .await
                .is_err()
        );
        assert!(
            storage
                .connect_nodes(workflow.id, a.id, b.id)
                .await
                .is_err()
        );
        assert!(
            storage
                .connect_nodes(workflow.id, b.id, a.id)
                .await
                .is_err()
        );
        storage.update_node(workflow.id, a.id, "A").await?;
        assert_eq!(storage.history_status(directory.path()).await?, pending);
        storage.update_node(workflow.id, a.id, "New branch").await?;
        let branched = storage.history_status(directory.path()).await?;
        assert!(branched.redo.is_none());
        assert_ne!(branched.undo.as_ref().expect("new edit").id, action.id);
        assert!(storage.redo(directory.path(), action.id).await.is_err());
        assert_eq!(storage.history_status(&other_project).await?, other_status);
        Ok(())
    })
}

#[test]
fn stale_selection_cannot_undo_a_different_action_or_a_replacement_redo_tail() -> anyhow::Result<()>
{
    run(async {
        let directory = tempfile::tempdir()?;
        let mut storage = Storage::open(&directory.path().join("workflows.db")).await?;
        let workflow = storage
            .create_workflow(directory.path(), "Shared clients")
            .await?;
        let node = storage.create_node(workflow.id, "Original").await?;
        let old = storage
            .history_status(directory.path())
            .await?
            .undo
            .expect("create edit");
        storage.update_node(workflow.id, node.id, "Newer").await?;
        let newer = storage.history_status(directory.path()).await?;
        assert!(storage.undo(directory.path(), old.id).await.is_err());
        assert_eq!(storage.history_status(directory.path()).await?, newer);
        let edit = newer.undo.expect("prompt edit");
        storage.undo(directory.path(), edit.id).await?;
        storage
            .update_node(workflow.id, node.id, "Replacement")
            .await?;
        let replacement = storage
            .history_status(directory.path())
            .await?
            .undo
            .expect("replacement edit");
        storage.undo(directory.path(), replacement.id).await?;
        assert!(storage.redo(directory.path(), edit.id).await.is_err());
        assert_eq!(storage.node(workflow.id, node.id).await?.prompt, "Original");
        Ok(())
    })
}
