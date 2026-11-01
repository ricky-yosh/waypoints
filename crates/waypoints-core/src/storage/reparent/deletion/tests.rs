use super::super::DependencyRecoveryAssessment;
use super::super::tests::{fixture, run};
use super::*;
use crate::run::RecoveryConfirmation;

#[test]
fn deletion_recovery_handles_both_crash_windows_without_repeating_jj() -> anyhow::Result<()> {
    run(async {
        for applied in [false, true] {
            let (directory, storage, workflow, node) = fixture().await?;
            let mut operation = storage.preflight_deletion(workflow, node).await?;
            storage.reserve_reparent(&mut operation).await?;
            if applied {
                jj::abandon_change(
                    directory.path(),
                    &operation.source,
                    &operation.native_operation,
                )
                .await?;
            }
            let native = jj::operation_id(directory.path()).await?;
            std::fs::write(directory.path().join("dirty.txt"), "retain me\n")?;
            drop(storage);
            let mut storage = Storage::open(&directory.path().join("workflow.db")).await?;
            let graph = storage.workflow_graph(workflow).await?;
            assert_eq!(
                storage
                    .inspect_dependency_recovery(operation.id)
                    .await?
                    .assessment,
                if applied {
                    DependencyRecoveryAssessment::Applied
                } else {
                    DependencyRecoveryAssessment::NotApplied
                }
            );
            assert_eq!(
                storage.workflow_graph(workflow).await?,
                graph,
                "inspection never changes edges"
            );
            assert!(
                storage
                    .recover_dependency_operation(operation.id, None)
                    .await
                    .is_err()
            );
            let result = storage
                .recover_dependency_operation(
                    operation.id,
                    Some(RecoveryConfirmation::ProcessesStopped),
                )
                .await?;
            assert_eq!(
                result.state,
                if applied {
                    DependencyOperationState::Completed
                } else {
                    DependencyOperationState::NotApplied
                }
            );
            assert_eq!(storage.node(workflow, node).await.is_err(), applied);
            assert!(
                storage
                    .workflow_graph(workflow)
                    .await?
                    .unresolved_dependency_operation
                    .is_none()
            );
            assert_eq!(jj::operation_id(directory.path()).await?, native);
            assert_eq!(
                std::fs::read_to_string(directory.path().join("dirty.txt"))?,
                "retain me\n"
            );
            let history = storage.history_status(directory.path()).await?;
            assert!(history.undo.is_none() && history.redo.is_none());
            assert_eq!(
                storage
                    .recover_dependency_operation(
                        operation.id,
                        Some(RecoveryConfirmation::ProcessesStopped)
                    )
                    .await?,
                result
            );
            if applied {
                // Delete the final root too; an empty based workflow is rooted.
                let root = storage.workflow_graph(workflow).await?.nodes[0].id;
                storage
                    .delete_node(
                        workflow,
                        root,
                        Some(UndoBoundaryAcknowledgement::AcceptUndoBoundary),
                    )
                    .await?;
                assert!(storage.workflow_graph(workflow).await?.nodes.is_empty());
            }
        }
        Ok(())
    })
}

#[test]
fn deletion_write_failure_rolls_back_edges_marker_and_release_and_retains_history_rows()
-> anyhow::Result<()> {
    run(async {
        let (directory, mut storage, workflow, node) = fixture().await?;
        let child = storage
            .create_node_from(workflow, "Child", &NodeParent::Node { node_id: node })
            .await?;
        storage
            .create_node_change_from_dependency(workflow, child.id)
            .await?;
        let connection = storage.database.connect()?;
        let mut operation = storage.preflight_deletion(workflow, node).await?;
        let historical = crate::run::Run {
            id: 1,
            workflow_id: workflow,
            node_id: node,
            prompt: "Captured assignment".to_owned(),
            directory: directory.path().join("historical"),
            change_id: operation.source.change_id.clone(),
            executable: "/historical/adapter".into(),
            session_id: Some("retained-session".to_owned()),
            continued_from: None,
            process_id: None,
            state: crate::run::RunState::Completed,
            upstream_inputs: None,
            snapshot: crate::run::SnapshotState::Saved {
                revision: operation.source.clone(),
            },
        };
        let record = serde_json::to_string(&historical)?;
        connection.execute("INSERT INTO agent_runs(node_id, directory, active, record) VALUES (?1, 'historical', 0, ?2)",
            turso::params![node, record.clone()]).await?;
        storage.reserve_reparent(&mut operation).await?;
        jj::abandon_change(
            directory.path(),
            &operation.source,
            &operation.native_operation,
        )
        .await?;
        let graph = storage.workflow_graph(workflow).await?;
        let native = jj::operation_id(directory.path()).await?;
        connection.execute("INSERT INTO dependency_operations(workflow_id, active, record) VALUES (?1, 0, '{}')", [workflow]).await?;
        connection
            .execute(
                "CREATE UNIQUE INDEX fail_delete_release ON dependency_operations(active)",
                (),
            )
            .await?;
        assert!(
            storage
                .recover_dependency_operation(
                    operation.id,
                    Some(RecoveryConfirmation::ProcessesStopped)
                )
                .await
                .is_err()
        );
        assert_eq!(storage.workflow_graph(workflow).await?, graph);
        assert!(
            storage.node(workflow, node).await.is_ok(),
            "no partial deletion marker"
        );
        assert!(storage.ensure_node_idle(child.id).await.is_err());
        connection
            .execute("DROP INDEX fail_delete_release", ())
            .await?;
        storage
            .recover_dependency_operation(
                operation.id,
                Some(RecoveryConfirmation::ProcessesStopped),
            )
            .await?;
        assert!(storage.node(workflow, node).await.is_err());
        let mut rows = connection.query(
            "SELECT n.prompt, r.record, c.change_id FROM nodes n
             JOIN agent_runs r ON r.node_id = n.id JOIN node_changes c ON c.node_id = n.id WHERE n.id = ?1",
            [node],
        ).await?;
        let row = rows.next().await?.expect("historical records are retained");
        assert_eq!(row.get::<String>(0)?, "B");
        assert_eq!(row.get::<String>(1)?, record);
        assert_eq!(row.get::<String>(2)?, operation.source.change_id);
        assert_eq!(
            storage.node_runs(workflow, node).await?,
            vec![historical.clone()]
        );
        assert_eq!(storage.run(historical.id).await?, historical);
        assert_eq!(
            storage.review_run(historical.id).await?.revision,
            operation.source
        );
        let mut unavailable = historical.clone();
        if let crate::run::SnapshotState::Saved { revision } = &mut unavailable.snapshot {
            revision.commit_id = "f".repeat(40);
        }
        storage.save_run(&unavailable).await?;
        assert!(storage.review_run(historical.id).await.is_err());
        storage.save_run(&historical).await?;
        assert!(storage.node_runs(workflow + 1, node).await.is_err());
        assert_eq!(jj::operation_id(directory.path()).await?, native);
        Ok(())
    })
}

#[test]
fn deletion_refuses_invalid_saved_workspace_and_busy_run_before_boundary() -> anyhow::Result<()> {
    run(async {
        let (directory, mut storage, workflow, node) = fixture().await?;
        let graph = storage.workflow_graph(workflow).await?;
        let history = storage.history_status(directory.path()).await?;
        let native = jj::operation_id(directory.path()).await?;
        let connection = storage.database.connect()?;
        connection.execute("INSERT INTO node_workspaces(node_id, repository_path, change_id, name, directory, ready)
            VALUES (?1, 'saved', 'saved', 'saved', 'saved', 1)", [node]).await?;
        assert!(
            storage
                .delete_node(
                    workflow,
                    node,
                    Some(UndoBoundaryAcknowledgement::AcceptUndoBoundary)
                )
                .await
                .expect_err("invalid saved workspace")
                .to_string()
                .contains("saved workspace")
        );
        connection
            .execute("DELETE FROM node_workspaces WHERE node_id = ?1", [node])
            .await?;
        connection.execute("INSERT INTO agent_runs(node_id, directory, active, record) VALUES (?1, 'busy', 1, '{}')", [node]).await?;
        assert!(
            storage
                .delete_node(
                    workflow,
                    node,
                    Some(UndoBoundaryAcknowledgement::AcceptUndoBoundary)
                )
                .await
                .is_err()
        );
        assert_eq!(storage.workflow_graph(workflow).await?, graph);
        assert_eq!(storage.history_status(directory.path()).await?, history);
        assert_eq!(jj::operation_id(directory.path()).await?, native);
        assert!(storage.dependency_operations(workflow).await?.is_empty());
        Ok(())
    })
}

#[test]
fn deletion_recovery_refuses_intervening_native_writes() -> anyhow::Result<()> {
    run(async {
        let (directory, mut storage, workflow, node) = fixture().await?;
        let mut operation = storage.preflight_deletion(workflow, node).await?;
        storage.reserve_reparent(&mut operation).await?;
        jj::abandon_change(
            directory.path(),
            &operation.source,
            &operation.native_operation,
        )
        .await?;
        jj::create_change(directory.path(), &operation.destination).await?;
        let graph = storage.workflow_graph(workflow).await?;
        let native = jj::operation_id(directory.path()).await?;
        assert!(matches!(
            storage
                .inspect_dependency_recovery(operation.id)
                .await?
                .assessment,
            DependencyRecoveryAssessment::Blocked { .. }
        ));
        assert!(
            storage
                .recover_dependency_operation(
                    operation.id,
                    Some(RecoveryConfirmation::ProcessesStopped)
                )
                .await
                .is_err()
        );
        assert_eq!(storage.workflow_graph(workflow).await?, graph);
        assert_eq!(jj::operation_id(directory.path()).await?, native);
        assert!(storage.ensure_node_idle(node).await.is_err());
        Ok(())
    })
}

#[test]
fn old_dependency_operation_records_remain_reparent_operations() -> anyhow::Result<()> {
    run(async {
        let (_directory, storage, workflow, node) = fixture().await?;
        let operation = storage
            .preflight_reparent(workflow, node, &NodeParent::WorkflowBase {})
            .await?;
        let mut legacy = serde_json::to_value(&operation)?;
        legacy
            .as_object_mut()
            .expect("operation object")
            .remove("kind");
        let decoded: DependencyOperation = serde_json::from_value(legacy)?;
        assert_eq!(decoded.kind, DependencyOperationKind::Reparent);
        assert_eq!(decoded, operation);
        Ok(())
    })
}
