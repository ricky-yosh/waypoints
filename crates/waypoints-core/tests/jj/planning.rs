use super::*;

#[test]
fn native_creation_uses_moved_prompt_dependency_and_then_blocks_metadata_rewind()
-> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        let mut storage = Storage::open(&repo.directory.path().join("planning.db")).await?;
        let workflow = storage
            .create_workflow_from(
                &repo.project,
                "Plan",
                &BaseSource::Revision {
                    id: "root()".into(),
                },
            )
            .await?;
        let a = storage
            .create_node_from(workflow.id, "A", &NodeParent::WorkflowBase {})
            .await?;
        storage
            .create_node_change_from_dependency(workflow.id, a.id)
            .await?;
        let b = storage
            .create_node_from(workflow.id, "B", &NodeParent::Node { node_id: a.id })
            .await?;
        let d = storage
            .create_node_from(workflow.id, "D", &NodeParent::WorkflowBase {})
            .await?;
        storage
            .create_node_change_from_dependency(workflow.id, d.id)
            .await?;
        let before = repo.operation()?;
        storage
            .move_prompt_node(workflow.id, b.id, &NodeParent::Node { node_id: d.id })
            .await?;
        assert_eq!(repo.operation()?, before);
        let edit = storage
            .history_status(&repo.project)
            .await?
            .undo
            .expect("move");
        let created = storage
            .create_node_change_from_dependency(workflow.id, b.id)
            .await?;
        let ChangeState::Current { revision } = created.state else {
            panic!("expected native revision");
        };
        let upstream = storage.refresh_node(workflow.id, d.id).await?;
        let ChangeState::Current { revision: parent } = upstream.state else {
            panic!("expected native parent");
        };
        assert_eq!(
            recorded_value(
                &repo,
                &revision.commit_id,
                "parents.map(|p| p.commit_id()).join(\" \")"
            )?,
            parent.commit_id
        );
        let native = repo.operation()?;
        let graph = storage.workflow_graph(workflow.id).await?;
        assert!(storage.delete_prompt_node(workflow.id, b.id).await.is_err());
        assert!(
            storage
                .move_prompt_node(workflow.id, b.id, &NodeParent::WorkflowBase {})
                .await
                .is_err()
        );
        assert!(storage.undo(&repo.project, edit.id).await.is_err());
        assert_eq!(repo.operation()?, native);
        assert_eq!(storage.workflow_graph(workflow.id).await?, graph);
        assert_eq!(
            storage
                .history_status(&repo.project)
                .await?
                .undo
                .expect("unchanged cursor")
                .id,
            edit.id
        );
        Ok(())
    })
}
