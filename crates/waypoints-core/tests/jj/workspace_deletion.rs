use super::*;

#[test]
fn workspace_deletion_reconnects_native_children_without_touching_any_checkout()
-> anyhow::Result<()> {
    run(async {
        let (repo, mut storage, workflow, nodes) = reparent_fixture(false).await?;
        let directory = repo.directory.path().join("kept");
        storage
            .create_node_workspace(workflow.id, nodes[2].id, &directory)
            .await?;
        fs::write(directory.join("review.txt"), "dirty source contents\n")?;
        fs::write(directory.join("untracked.txt"), "untracked bytes\n")?;
        let checkout = fs::read(directory.join(".jj/working_copy/checkout"))?;
        let tree = fs::read(directory.join(".jj/working_copy/tree_state"))?;
        let original = fs::read(repo.project.join(".jj/working_copy/checkout"))?;
        let child = storage
            .refresh_node(workflow.id, nodes[3].id)
            .await?
            .association
            .expect("child");
        let result = storage
            .delete_node(
                workflow.id,
                nodes[2].id,
                Some(UndoBoundaryAcknowledgement::AcceptUndoBoundary),
            )
            .await?;
        assert_eq!(result.state, DependencyOperationState::Completed);
        assert_eq!(result.after.len(), 1);
        let next = storage.refresh_node(workflow.id, nodes[3].id).await?;
        assert_eq!(next.association.as_ref(), Some(&child));
        let ChangeState::Current { revision } = next.state else {
            panic!("child must remain current");
        };
        assert_eq!(
            recorded_parents(&repo, &revision.commit_id)?,
            vec![
                storage
                    .refresh_node(workflow.id, nodes[1].id)
                    .await?
                    .association
                    .expect("parent")
                    .change_id
            ]
        );
        let files = repo.jj(&[
            "--ignore-working-copy",
            "--at-op=@",
            "file",
            "list",
            "-r",
            &revision.commit_id,
        ])?;
        assert!(!files.lines().any(|line| line == "review.txt"));
        assert!(files.lines().any(|line| line == "followup.txt"));
        assert_eq!(
            fs::read_to_string(directory.join("review.txt"))?,
            "dirty source contents\n"
        );
        assert_eq!(
            fs::read_to_string(directory.join("untracked.txt"))?,
            "untracked bytes\n"
        );
        assert_eq!(
            fs::read(directory.join(".jj/working_copy/checkout"))?,
            checkout
        );
        assert_eq!(
            fs::read(directory.join(".jj/working_copy/tree_state"))?,
            tree
        );
        assert_eq!(
            fs::read(repo.project.join(".jj/working_copy/checkout"))?,
            original
        );
        assert!(storage.history_status(&repo.project).await?.undo.is_none());
        Ok(())
    })
}

#[test]
fn unsafe_workspace_states_refuse_before_native_mutation_or_undo_boundary() -> anyhow::Result<()> {
    run(async {
        for case in ["missing", "switched", "additional"] {
            let (repo, mut storage, workflow, nodes) = reparent_fixture(false).await?;
            let directory = repo.directory.path().join("kept");
            storage
                .create_node_workspace(workflow.id, nodes[2].id, &directory)
                .await?;
            match case {
                "missing" => fs::remove_dir_all(&directory)?,
                "switched" => {
                    let parent = storage
                        .refresh_node(workflow.id, nodes[1].id)
                        .await?
                        .association
                        .expect("parent");
                    repo.jj_at(&directory, &["edit", &parent.change_id])?;
                }
                _ => {
                    let source = storage
                        .refresh_node(workflow.id, nodes[2].id)
                        .await?
                        .association
                        .expect("source");
                    let extra = repo.directory.path().join("extra");
                    repo.jj(&[
                        "--ignore-working-copy",
                        "workspace",
                        "add",
                        "--name",
                        "extra",
                        "--no-colocate",
                        "--sparse-patterns",
                        "full",
                        "-r",
                        &source.change_id,
                        extra.to_str().expect("path"),
                    ])?;
                    repo.jj_at(
                        &extra,
                        &["--ignore-working-copy", "edit", &source.change_id],
                    )?;
                }
            }
            let native = repo.operation()?;
            let graph = storage.workflow_graph(workflow.id).await?;
            let history = storage.history_status(&repo.project).await?;
            assert!(
                storage
                    .delete_node(
                        workflow.id,
                        nodes[2].id,
                        Some(UndoBoundaryAcknowledgement::AcceptUndoBoundary)
                    )
                    .await
                    .is_err(),
                "{case}"
            );
            assert_eq!(repo.operation()?, native, "{case}");
            assert_eq!(storage.workflow_graph(workflow.id).await?, graph, "{case}");
            assert_eq!(
                storage.history_status(&repo.project).await?,
                history,
                "{case}"
            );
            assert!(storage.dependency_operations(workflow.id).await?.is_empty());
        }
        Ok(())
    })
}
