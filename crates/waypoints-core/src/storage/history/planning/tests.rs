use super::*;
use crate::jj::Revision;
use crate::workflow::{BaseSource, WorkflowBase};

fn run(test: impl std::future::Future<Output = anyhow::Result<()>>) -> anyhow::Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .build()?
        .block_on(test)
}

async fn fixture() -> anyhow::Result<(tempfile::TempDir, Storage, i64, Vec<Node>)> {
    let directory = tempfile::tempdir()?;
    let mut storage = Storage::open(&directory.path().join("planning.db")).await?;
    // This intentionally is not a jj repository: all planning and history work must be SQL-only.
    let workflow = storage
        .create_workflow_record(
            directory.path(),
            "Plan",
            Some(WorkflowBase {
                repository_path: directory.path().to_owned(),
                source: BaseSource::Revision {
                    id: "root()".into(),
                },
                revision: Revision {
                    change_id: "z".repeat(32),
                    commit_id: "0".repeat(40),
                    description: String::new(),
                    conflicted: false,
                },
            }),
        )
        .await?;
    let a = storage
        .create_node_from(workflow.id, "A", &NodeParent::WorkflowBase {})
        .await?;
    let b = storage
        .create_node_from(
            workflow.id,
            "B\nexact prompt",
            &NodeParent::Node { node_id: a.id },
        )
        .await?;
    let c = storage
        .create_node_from(workflow.id, "C", &NodeParent::Node { node_id: b.id })
        .await?;
    let d = storage
        .create_node_from(workflow.id, "D", &NodeParent::WorkflowBase {})
        .await?;
    let e = storage
        .create_node_from(workflow.id, "E", &NodeParent::Node { node_id: b.id })
        .await?;
    let f = storage
        .create_node_from(workflow.id, "F", &NodeParent::Node { node_id: c.id })
        .await?;
    Ok((directory, storage, workflow.id, vec![a, b, c, d, e, f]))
}

#[test]
fn move_round_trips_and_rejects_cycles_foreign_parents_and_stale_history() -> anyhow::Result<()> {
    run(async {
        let (dir, mut storage, workflow, nodes) = fixture().await?;
        let before = storage.workflow_graph(workflow).await?;
        for parent in [
            NodeParent::Node {
                node_id: nodes[1].id,
            },
            NodeParent::Node {
                node_id: nodes[5].id,
            },
            NodeParent::Node { node_id: 999 },
        ] {
            let history = storage.history_status(dir.path()).await?;
            assert!(
                storage
                    .move_prompt_node(workflow, nodes[1].id, &parent)
                    .await
                    .is_err()
            );
            assert_eq!(storage.workflow_graph(workflow).await?, before);
            assert_eq!(storage.history_status(dir.path()).await?, history);
        }
        let other = storage.create_workflow(dir.path(), "Other").await?;
        let foreign = storage.create_node(other.id, "Foreign").await?;
        assert!(
            storage
                .move_prompt_node(
                    workflow,
                    nodes[1].id,
                    &NodeParent::Node {
                        node_id: foreign.id
                    }
                )
                .await
                .is_err()
        );
        assert!(
            storage
                .delete_prompt_node(other.id, nodes[1].id)
                .await
                .is_err()
        );
        assert!(
            storage
                .delete_prompt_node(other.id, foreign.id)
                .await
                .is_err()
        );
        let moved = storage
            .move_prompt_node(
                workflow,
                nodes[1].id,
                &NodeParent::Node {
                    node_id: nodes[3].id,
                },
            )
            .await?;
        assert_ne!(moved, before);
        assert_eq!(
            parents(&moved, nodes[1].id),
            vec![NodeParent::Node {
                node_id: nodes[3].id
            }]
        );
        let edit = storage
            .history_status(dir.path())
            .await?
            .undo
            .expect("move");
        assert_eq!(edit.label, "Move prompt node");
        assert!(storage.undo(dir.path(), edit.id - 1).await.is_err());
        storage.undo(dir.path(), edit.id).await?;
        assert_eq!(storage.workflow_graph(workflow).await?, before);
        let history = storage.history_status(dir.path()).await?;
        storage
            .move_prompt_node(
                workflow,
                nodes[1].id,
                &NodeParent::Node {
                    node_id: nodes[0].id,
                },
            )
            .await?;
        assert_eq!(
            storage.history_status(dir.path()).await?,
            history,
            "no-op must preserve redo"
        );
        drop(storage);
        let mut storage = Storage::open(&dir.path().join("planning.db")).await?;
        storage.redo(dir.path(), edit.id).await?;
        assert_eq!(storage.workflow_graph(workflow).await?, moved);
        storage
            .move_prompt_node(workflow, nodes[1].id, &NodeParent::WorkflowBase {})
            .await?;
        assert!(
            storage
                .workflow_graph(workflow)
                .await?
                .base_nodes
                .contains(&nodes[1].id)
        );
        Ok(())
    })
}

#[test]
fn deletion_restores_exact_node_children_and_deduplicates_inherited_attachments()
-> anyhow::Result<()> {
    run(async {
        let (dir, mut storage, workflow, nodes) = fixture().await?;
        let connection = storage.database.connect()?;
        // A child can already depend on its grandparent; deletion must not duplicate that edge.
        connection
            .execute(
                "INSERT INTO connections VALUES (?1, ?2, ?3)",
                [workflow, nodes[0].id, nodes[2].id],
            )
            .await?;
        let before = storage.workflow_graph(workflow).await?;
        let deleted = storage.delete_prompt_node(workflow, nodes[1].id).await?;
        assert!(storage.node(workflow, nodes[1].id).await.is_err());
        assert_eq!(
            parents(&deleted, nodes[2].id),
            vec![NodeParent::Node {
                node_id: nodes[0].id
            }]
        );
        assert_eq!(
            parents(&deleted, nodes[4].id),
            vec![NodeParent::Node {
                node_id: nodes[0].id
            }]
        );
        assert_eq!(
            parents(&deleted, nodes[5].id),
            parents(&before, nodes[5].id)
        );
        let edit = storage
            .history_status(dir.path())
            .await?
            .undo
            .expect("delete");
        assert_eq!(edit.label, "Delete prompt node");
        drop(storage);
        let mut storage = Storage::open(&dir.path().join("planning.db")).await?;
        storage.undo(dir.path(), edit.id).await?;
        assert_eq!(storage.workflow_graph(workflow).await?, before);
        assert_eq!(storage.node(workflow, nodes[1].id).await?, nodes[1]);
        storage.redo(dir.path(), edit.id).await?;
        assert_eq!(storage.workflow_graph(workflow).await?, deleted);
        storage.undo(dir.path(), edit.id).await?;
        storage.delete_prompt_node(workflow, nodes[0].id).await?;
        let rooted = storage.workflow_graph(workflow).await?;
        assert!(rooted.base_nodes.contains(&nodes[1].id));
        assert!(rooted.base_nodes.contains(&nodes[2].id));
        let root_edit = storage
            .history_status(dir.path())
            .await?
            .undo
            .expect("root delete");
        storage.undo(dir.path(), root_edit.id).await?;
        assert_eq!(storage.workflow_graph(workflow).await?, before);
        storage.delete_prompt_node(workflow, nodes[3].id).await?;
        let leaf_edit = storage
            .history_status(dir.path())
            .await?
            .undo
            .expect("leaf delete");
        storage.undo(dir.path(), leaf_edit.id).await?;
        assert_eq!(storage.workflow_graph(workflow).await?, before);
        let new = storage
            .create_node_from(workflow, "New", &NodeParent::WorkflowBase {})
            .await?;
        assert!(new.id > nodes[5].id);
        Ok(())
    })
}

#[test]
fn every_native_reference_in_descendants_blocks_edits_and_history_replay() -> anyhow::Result<()> {
    run(async {
        let (dir, mut storage, workflow, nodes) = fixture().await?;
        let connection = storage.database.connect()?;
        for (insert, table) in [
            (
                "INSERT INTO node_changes VALUES (?1, 'repo', 'change')",
                "node_changes",
            ),
            (
                "INSERT INTO node_change_creations VALUES (?1)",
                "node_change_creations",
            ),
            (
                "INSERT INTO node_workspaces VALUES (?1, 'repo', 'change', 'name', 'dir', 0)",
                "node_workspaces",
            ),
            (
                "INSERT INTO agent_runs(node_id,directory,active,record) VALUES (?1, 'dir', 0, '{}')",
                "agent_runs",
            ),
        ] {
            connection.execute(insert, [nodes[5].id]).await?;
            let graph = storage.workflow_graph(workflow).await?;
            let history = storage.history_status(dir.path()).await?;
            assert!(
                storage
                    .move_prompt_node(workflow, nodes[1].id, &NodeParent::WorkflowBase {})
                    .await
                    .is_err(),
                "{table}"
            );
            assert!(
                storage
                    .delete_prompt_node(workflow, nodes[1].id)
                    .await
                    .is_err(),
                "{table}"
            );
            assert_eq!(storage.workflow_graph(workflow).await?, graph);
            assert_eq!(storage.history_status(dir.path()).await?, history);
            connection
                .execute(&format!("DELETE FROM {table}"), ())
                .await?;
        }
        storage
            .move_prompt_node(workflow, nodes[1].id, &NodeParent::WorkflowBase {})
            .await?;
        let edit = storage
            .history_status(dir.path())
            .await?
            .undo
            .expect("move");
        connection
            .execute(
                "INSERT INTO node_change_creations VALUES (?1)",
                [nodes[5].id],
            )
            .await?;
        let graph = storage.workflow_graph(workflow).await?;
        assert!(storage.undo(dir.path(), edit.id).await.is_err());
        assert_eq!(storage.workflow_graph(workflow).await?, graph);
        connection
            .execute("DELETE FROM node_change_creations", ())
            .await?;
        storage.undo(dir.path(), edit.id).await?;
        connection
            .execute(
                "INSERT INTO node_change_creations VALUES (?1)",
                [nodes[1].id],
            )
            .await?;
        assert!(storage.redo(dir.path(), edit.id).await.is_err());
        connection
            .execute("DELETE FROM node_change_creations", ())
            .await?;
        storage.delete_prompt_node(workflow, nodes[1].id).await?;
        let edit = storage
            .history_status(dir.path())
            .await?
            .undo
            .expect("delete");
        connection
            .execute(
                "INSERT INTO node_change_creations VALUES (?1)",
                [nodes[5].id],
            )
            .await?;
        assert!(storage.undo(dir.path(), edit.id).await.is_err());
        Ok(())
    })
}

#[test]
fn history_and_cursor_failures_roll_back_complete_topology_and_redo_tail() -> anyhow::Result<()> {
    run(async {
        let (dir, mut storage, workflow, nodes) = fixture().await?;
        storage
            .move_prompt_node(workflow, nodes[1].id, &NodeParent::WorkflowBase {})
            .await?;
        let edit = storage
            .history_status(dir.path())
            .await?
            .undo
            .expect("move");
        storage.undo(dir.path(), edit.id).await?;
        let connection = storage.database.connect()?;
        let graph = storage.workflow_graph(workflow).await?;
        let history = storage.history_status(dir.path()).await?;
        storage.redo(dir.path(), edit.id).await?;
        connection
            .execute(
                "CREATE UNIQUE INDEX fail_planning ON workflow_edits(version) WHERE version = 3",
                (),
            )
            .await?;
        let moved = storage.workflow_graph(workflow).await?;
        let moved_history = storage.history_status(dir.path()).await?;
        assert!(
            storage
                .delete_prompt_node(workflow, nodes[1].id)
                .await
                .is_err()
        );
        assert!(
            storage
                .move_prompt_node(
                    workflow,
                    nodes[1].id,
                    &NodeParent::Node {
                        node_id: nodes[3].id
                    }
                )
                .await
                .is_err()
        );
        assert_eq!(storage.workflow_graph(workflow).await?, moved);
        assert_eq!(storage.history_status(dir.path()).await?, moved_history);
        connection.execute("DROP INDEX fail_planning", ()).await?;
        let mut rows = connection
            .query("SELECT cursor FROM workflow_history_heads", ())
            .await?;
        let cursor: i64 = rows.next().await?.expect("cursor").get(0)?;
        drop(rows);
        connection
            .execute(
                "INSERT INTO workflow_history_heads VALUES ('sentinel', ?1)",
                [cursor - 1],
            )
            .await?;
        connection
            .execute(
                "CREATE UNIQUE INDEX fail_cursor ON workflow_history_heads(cursor)",
                (),
            )
            .await?;
        assert!(storage.undo(dir.path(), edit.id).await.is_err());
        assert_eq!(storage.workflow_graph(workflow).await?, moved);
        assert_eq!(storage.history_status(dir.path()).await?, moved_history);
        connection.execute("DROP INDEX fail_cursor", ()).await?;
        storage.undo(dir.path(), edit.id).await?;
        assert_eq!(storage.workflow_graph(workflow).await?, graph);
        assert_eq!(storage.history_status(dir.path()).await?, history);
        // Fail after the append has deleted the redo tail and mutated the topology.
        connection
            .execute(
                "UPDATE workflow_history_heads SET cursor = ?1 WHERE project_path = 'sentinel'",
                [cursor],
            )
            .await?;
        connection
            .execute(
                "CREATE UNIQUE INDEX fail_cursor ON workflow_history_heads(cursor)",
                (),
            )
            .await?;
        assert!(
            storage
                .delete_prompt_node(workflow, nodes[1].id)
                .await
                .is_err()
        );
        assert!(
            storage
                .move_prompt_node(workflow, nodes[1].id, &NodeParent::WorkflowBase {})
                .await
                .is_err()
        );
        assert!(storage.redo(dir.path(), edit.id).await.is_err());
        assert_eq!(storage.workflow_graph(workflow).await?, graph);
        assert_eq!(storage.history_status(dir.path()).await?, history);
        Ok(())
    })
}

#[test]
fn ambiguous_parents_and_changed_local_state_are_refused_without_partial_replay()
-> anyhow::Result<()> {
    run(async {
        let (dir, mut storage, workflow, nodes) = fixture().await?;
        let connection = storage.database.connect()?;
        connection
            .execute(
                "INSERT INTO connections VALUES (?1, ?2, ?3)",
                [workflow, nodes[3].id, nodes[1].id],
            )
            .await?;
        let graph = storage.workflow_graph(workflow).await?;
        let history = storage.history_status(dir.path()).await?;
        assert!(
            storage
                .delete_prompt_node(workflow, nodes[1].id)
                .await
                .is_err()
        );
        assert!(
            storage
                .move_prompt_node(workflow, nodes[1].id, &NodeParent::WorkflowBase {})
                .await
                .is_err()
        );
        assert_eq!(storage.workflow_graph(workflow).await?, graph);
        assert_eq!(storage.history_status(dir.path()).await?, history);
        connection
            .execute(
                "DELETE FROM connections WHERE source_id = ?1 AND target_id = ?2",
                [nodes[3].id, nodes[1].id],
            )
            .await?;
        storage.delete_prompt_node(workflow, nodes[1].id).await?;
        let edit = storage
            .history_status(dir.path())
            .await?
            .undo
            .expect("delete");
        // A new unrelated dependency on a surviving child must not be silently overwritten.
        connection
            .execute(
                "INSERT INTO connections VALUES (?1, ?2, ?3)",
                [workflow, nodes[3].id, nodes[2].id],
            )
            .await?;
        let changed = storage.workflow_graph(workflow).await?;
        assert!(storage.undo(dir.path(), edit.id).await.is_err());
        assert_eq!(storage.workflow_graph(workflow).await?, changed);
        assert_eq!(
            storage
                .history_status(dir.path())
                .await?
                .undo
                .expect("unchanged cursor")
                .id,
            edit.id
        );
        Ok(())
    })
}

#[test]
fn deleting_the_last_root_and_undoing_creation_history_preserves_identities() -> anyhow::Result<()>
{
    run(async {
        let (dir, mut storage, workflow, nodes) = fixture().await?;
        // Undo all creations except the first root, using the existing history implementation.
        for _ in 1..nodes.len() {
            let edit = storage
                .history_status(dir.path())
                .await?
                .undo
                .expect("create");
            storage.undo(dir.path(), edit.id).await?;
        }
        let before = storage.workflow_graph(workflow).await?;
        let empty = storage.delete_prompt_node(workflow, nodes[0].id).await?;
        assert!(
            empty.nodes.is_empty() && empty.base_nodes.is_empty() && empty.connections.is_empty()
        );
        let edit = storage
            .history_status(dir.path())
            .await?
            .undo
            .expect("delete");
        storage.undo(dir.path(), edit.id).await?;
        assert_eq!(storage.workflow_graph(workflow).await?, before);
        let create = storage
            .history_status(dir.path())
            .await?
            .undo
            .expect("create");
        storage.undo(dir.path(), create.id).await?;
        storage.redo(dir.path(), create.id).await?;
        storage.redo(dir.path(), edit.id).await?;
        let replacement = storage
            .create_node_from(workflow, "Replacement", &NodeParent::WorkflowBase {})
            .await?;
        assert!(
            replacement.id > nodes[5].id,
            "deleted and undone identities must never be reused"
        );
        Ok(())
    })
}
