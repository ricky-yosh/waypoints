use super::*;

#[test]
fn failed_multi_parent_history_write_restores_all_attachments_redo_and_identity()
-> anyhow::Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .build()?
        .block_on(async {
            let directory = tempfile::tempdir()?;
            let project = directory.path();
            let mut storage = Storage::open(&project.join("workflow.db")).await?;
            let base = crate::workflow::WorkflowBase {
                repository_path: project.to_owned(),
                source: crate::workflow::BaseSource::Revision {
                    id: "root()".to_owned(),
                },
                revision: crate::jj::Revision {
                    change_id: "k".repeat(32),
                    commit_id: "0".repeat(40),
                    description: String::new(),
                    conflicted: false,
                },
            };
            let workflow = storage
                .create_workflow_record(project, "Atomic", Some(base))
                .await?;
            let mut parents = Vec::new();
            for prompt in ["A", "B", "C"] {
                let node = storage
                    .create_node_from(workflow.id, prompt, &NodeParent::WorkflowBase {})
                    .await?;
                parents.push(NodeParent::Node { node_id: node.id });
            }
            storage
                .create_node_from_parents(workflow.id, "First", &parents[..2])
                .await?;
            let second = storage
                .create_node_from_parents(workflow.id, "Second", &parents)
                .await?;
            let edit = storage
                .history_status(project)
                .await?
                .undo
                .expect("second creation");
            let pending = storage.undo(project, edit.id).await?;
            let graph = storage.workflow_graph(workflow.id).await?;
            let connection = storage.database.connect()?;
            connection
                .execute(
                    &format!(
                        "CREATE UNIQUE INDEX fail_multi_parent ON workflow_edits(version)
             WHERE version = 5 AND id <> {}",
                        edit.id
                    ),
                    (),
                )
                .await?;
            assert!(
                storage
                    .create_node_from_parents(workflow.id, "Must roll back", &parents)
                    .await
                    .is_err()
            );
            assert_eq!(storage.workflow_graph(workflow.id).await?, graph);
            assert_eq!(storage.history_status(project).await?, pending);
            connection
                .execute("DROP INDEX fail_multi_parent", ())
                .await?;
            storage.redo(project, edit.id).await?;
            assert_eq!(storage.node(workflow.id, second.id).await?, second);
            assert_eq!(
                storage
                    .create_node_from_parents(workflow.id, "Next", &parents)
                    .await?
                    .id,
                second.id + 1
            );
            Ok(())
        })
}
