use super::*;

async fn branches(
    conflicting: bool,
) -> anyhow::Result<(Repository, Storage, Workflow, Vec<Node>, Vec<String>)> {
    let repo = Repository::new()?;
    let mut storage = Storage::open(&repo.directory.path().join("workflow.db")).await?;
    let workflow = storage
        .create_workflow_from(
            &repo.project,
            "Combine work",
            &BaseSource::Revision {
                id: "root()".to_owned(),
            },
        )
        .await?;
    let mut nodes = Vec::new();
    let mut commits = Vec::new();
    for name in ["api", "ui", "database"] {
        repo.jj(&["new", "root()"])?;
        let file = if conflicting {
            "shared.txt".to_owned()
        } else {
            format!("{name}.txt")
        };
        fs::write(repo.project.join(file), format!("{name}\n"))?;
        repo.jj(&["describe", "-m", name])?;
        let node = storage
            .create_node_from(workflow.id, name, &NodeParent::WorkflowBase {})
            .await?;
        storage
            .associate_node(workflow.id, node.id, &repo.value("change_id")?)
            .await?;
        commits.push(repo.value("commit_id")?);
        nodes.push(node);
    }
    Ok((repo, storage, workflow, nodes, commits))
}

#[test]
fn two_and_three_parent_nodes_merge_recorded_code_and_preserve_checkout() -> anyhow::Result<()> {
    run(async {
        let (repo, mut storage, workflow, nodes, commits) = branches(false).await?;
        let checkout = recorded_value(&repo, "@", "commit_id")?;
        fs::write(repo.project.join("dirty.txt"), "not recorded\n")?;
        for count in [2, 3] {
            // Reverse selection order deliberately: graph storage has stable ID
            // order; undo must compare parent sets rather than request ordering.
            let parents: Vec<_> = nodes[..count]
                .iter()
                .rev()
                .map(|node| NodeParent::Node { node_id: node.id })
                .collect();
            let before = storage.workflow_graph(workflow.id).await?;
            let merge = storage
                .create_node_from_parents(workflow.id, "Integrate and test", &parents)
                .await?;
            let graph = storage.workflow_graph(workflow.id).await?;
            let edit = storage
                .history_status(&repo.project)
                .await?
                .undo
                .expect("creation");
            storage.undo(&repo.project, edit.id).await?;
            assert_eq!(storage.workflow_graph(workflow.id).await?, before);
            drop(storage);
            storage = Storage::open(&repo.directory.path().join("workflow.db")).await?;
            storage.redo(&repo.project, edit.id).await?;
            assert_eq!(storage.workflow_graph(workflow.id).await?, graph);
            let operation = repo.operation()?;
            assert!(
                storage
                    .create_node_change(workflow.id, merge.id, &commits[0])
                    .await
                    .is_err(),
                "an explicit single base cannot discard merge parents"
            );
            assert_eq!(repo.operation()?, operation);
            let created = storage
                .create_node_change_from_dependency(workflow.id, merge.id)
                .await?;
            let ChangeState::Current { revision } = created.state else {
                panic!("expected current merge")
            };
            assert!(!revision.conflicted);
            let review = storage.review_node(workflow.id, merge.id).await?;
            let mut actual = review.parent_commit_ids;
            actual.sort();
            let mut expected = commits[..count].to_vec();
            expected.sort();
            assert_eq!(
                actual, expected,
                "every selected native parent must be retained"
            );
            let destination = repo.directory.path().join(format!("merge-{count}"));
            storage
                .create_node_workspace(workflow.id, merge.id, &destination)
                .await?;
            for name in &["api", "ui", "database"][..count] {
                assert_eq!(
                    fs::read_to_string(destination.join(format!("{name}.txt")))?,
                    format!("{name}\n")
                );
            }
            assert!(!destination.join("dirty.txt").exists());
            assert_eq!(recorded_value(&repo, "@", "commit_id")?, checkout);
            assert_eq!(
                fs::read_to_string(repo.project.join("dirty.txt"))?,
                "not recorded\n"
            );
            let pending = storage.history_status(&repo.project).await?;
            assert!(
                storage.undo(&repo.project, edit.id).await.is_err(),
                "native merge references must block metadata deletion"
            );
            assert_eq!(storage.workflow_graph(workflow.id).await?, graph);
            assert_eq!(storage.history_status(&repo.project).await?, pending);
            assert!(
                storage
                    .create_node_change_from_dependency(workflow.id, merge.id)
                    .await
                    .is_err(),
                "creation must not repeat"
            );
        }
        Ok(())
    })
}

#[test]
fn conflicting_two_and_three_parent_merges_are_saved_and_materialized_not_silently_resolved()
-> anyhow::Result<()> {
    run(async {
        let (repo, mut storage, workflow, nodes, _) = branches(true).await?;
        for count in [2, 3] {
            let parents: Vec<_> = nodes[..count]
                .iter()
                .map(|node| NodeParent::Node { node_id: node.id })
                .collect();
            let merge = storage
                .create_node_from_parents(workflow.id, "Resolve integration", &parents)
                .await?;
            let created = storage
                .create_node_change_from_dependency(workflow.id, merge.id)
                .await?;
            assert!(
                matches!(created.state, ChangeState::Current { revision } if revision.conflicted)
            );
            let destination = repo.directory.path().join(format!("conflict-{count}"));
            storage
                .create_node_workspace(workflow.id, merge.id, &destination)
                .await?;
            storage.verify_node_workspace(workflow.id, merge.id).await?;
            assert!(
                fs::read_to_string(destination.join("shared.txt"))?.contains("<<<<<<<"),
                "native conflict markers must remain for explicit resolution"
            );
            assert!(
                storage
                    .review_node(workflow.id, merge.id)
                    .await?
                    .revision
                    .conflicted
            );
        }
        drop(storage);
        let storage = Storage::open(&repo.directory.path().join("workflow.db")).await?;
        assert!(
            storage
                .review_node(workflow.id, 5)
                .await?
                .revision
                .conflicted
        );
        Ok(())
    })
}

#[test]
fn invalid_parent_sets_and_unavailable_native_parents_change_nothing() -> anyhow::Result<()> {
    run(async {
        let (repo, mut storage, workflow, nodes, _) = branches(false).await?;
        let foreign = storage
            .create_workflow_from(
                &repo.project,
                "Other",
                &BaseSource::Revision {
                    id: "root()".to_owned(),
                },
            )
            .await?;
        let foreign_node = storage
            .create_node_from(foreign.id, "Foreign", &NodeParent::WorkflowBase {})
            .await?;
        let parent = NodeParent::Node {
            node_id: nodes[0].id,
        };
        let before = storage.workflow_graph(workflow.id).await?;
        let history = storage.history_status(&repo.project).await?;
        let operation = repo.operation()?;
        for parents in [
            vec![],
            vec![parent.clone(), parent.clone()],
            vec![
                parent.clone(),
                NodeParent::Node {
                    node_id: foreign_node.id,
                },
            ],
            vec![parent.clone(), NodeParent::Node { node_id: i64::MAX }],
            vec![NodeParent::WorkflowBase {}, parent.clone()],
        ] {
            assert!(
                storage
                    .create_node_from_parents(workflow.id, "Invalid", &parents)
                    .await
                    .is_err(),
                "{parents:?}"
            );
            assert_eq!(storage.workflow_graph(workflow.id).await?, before);
            assert_eq!(storage.history_status(&repo.project).await?, history);
            assert_eq!(repo.operation()?, operation);
        }
        let missing = storage
            .create_node_from(
                workflow.id,
                "No native change",
                &NodeParent::WorkflowBase {},
            )
            .await?;
        let merge = storage
            .create_node_from_parents(
                workflow.id,
                "Not ready",
                &[
                    parent,
                    NodeParent::Node {
                        node_id: missing.id,
                    },
                ],
            )
            .await?;
        assert!(
            storage
                .create_node_change_from_dependency(workflow.id, merge.id)
                .await
                .is_err()
        );
        assert_eq!(repo.operation()?, operation);
        assert!(
            storage
                .refresh_node(workflow.id, merge.id)
                .await?
                .association
                .is_none()
        );
        // A rejected preflight must not leave an uncertain-creation guard.
        storage
            .create_node_change_from_dependency(workflow.id, missing.id)
            .await?;
        storage
            .create_node_change_from_dependency(workflow.id, merge.id)
            .await?;
        Ok(())
    })
}
