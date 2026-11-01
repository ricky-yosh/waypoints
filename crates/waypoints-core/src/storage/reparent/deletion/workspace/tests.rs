use super::*;
use crate::storage::DependencyRecoveryAssessment;
use crate::storage::reparent::tests::{fixture, run};

#[test]
fn final_write_failure_keeps_allocation_and_reservation_until_metadata_recovery()
-> anyhow::Result<()> {
    run(async {
        let (repository, mut storage, workflow, node) = fixture().await?;
        let kept = tempfile::tempdir()?;
        let directory = kept.path().join("workspace");
        storage
            .create_node_workspace(workflow, node, &directory)
            .await?;
        let mut operation = storage.preflight_deletion(workflow, node).await?;
        storage.reserve_reparent(&mut operation).await?;
        jj::abandon_change(
            repository.path(),
            &operation.source,
            &operation.native_operation,
        )
        .await?;
        storage
            .checkpoint_abandoned_deletion(&mut operation)
            .await?;
        storage.forget_deletion_workspace(&operation).await?;
        let native = jj::operation_id(repository.path()).await?;
        let graph = storage.workflow_graph(workflow).await?;
        let connection = storage.database.connect()?;
        connection
            .execute(
                "INSERT INTO dependency_operations(workflow_id,active,record) VALUES (?1,0,'{}')",
                [workflow],
            )
            .await?;
        connection
            .execute(
                "CREATE UNIQUE INDEX fail_release ON dependency_operations(active)",
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
        storage.verify_deletion_allocation(&operation).await?;
        assert!(storage.node(workflow, node).await.is_ok());
        connection.execute("DROP INDEX fail_release", ()).await?;
        storage
            .recover_dependency_operation(
                operation.id,
                Some(RecoveryConfirmation::ProcessesStopped),
            )
            .await?;
        assert!(storage.node(workflow, node).await.is_err());
        storage.verify_deletion_allocation(&operation).await?;
        assert_eq!(jj::operation_id(repository.path()).await?, native);
        assert!(directory.is_dir());
        Ok(())
    })
}

#[test]
fn crash_windows_require_explicit_resume_and_preserve_every_workspace_file() -> anyhow::Result<()> {
    run(async {
        for stage in 0..4 {
            let (repository, mut storage, workflow, node) = fixture().await?;
            let kept = tempfile::tempdir()?;
            let directory = kept.path().join("kept-workspace");
            storage
                .create_node_workspace(workflow, node, &directory)
                .await?;
            std::fs::write(directory.join("dirty.txt"), "keep every byte\n")?;
            let checkout = std::fs::read(directory.join(".jj/working_copy/checkout"))?;
            let tree = std::fs::read(directory.join(".jj/working_copy/tree_state"))?;
            let mut operation = storage.preflight_deletion(workflow, node).await?;
            storage.reserve_reparent(&mut operation).await?;
            if stage >= 1 {
                jj::abandon_change(
                    repository.path(),
                    &operation.source,
                    &operation.native_operation,
                )
                .await?;
            }
            if stage >= 2 {
                storage
                    .checkpoint_abandoned_deletion(&mut operation)
                    .await?;
            }
            if stage == 3 {
                storage.forget_deletion_workspace(&operation).await?;
            }
            let native = jj::operation_id(repository.path()).await?;
            drop(storage);
            let mut storage = Storage::open(&repository.path().join("workflow.db")).await?;
            let expected = match stage {
                0 => DependencyRecoveryAssessment::NotApplied,
                3 => DependencyRecoveryAssessment::Applied,
                _ => DependencyRecoveryAssessment::DeletionAbandoned,
            };
            assert_eq!(
                storage
                    .inspect_dependency_recovery(operation.id)
                    .await?
                    .assessment,
                expected
            );
            assert_eq!(
                jj::operation_id(repository.path()).await?,
                native,
                "inspection is read-only"
            );
            assert!(
                storage
                    .resume_node_deletion(operation.id, None)
                    .await
                    .is_err()
            );
            if stage == 1 || stage == 2 {
                assert!(
                    storage
                        .recover_dependency_operation(
                            operation.id,
                            Some(RecoveryConfirmation::ProcessesStopped)
                        )
                        .await
                        .is_err(),
                    "metadata recovery must not authorize remaining deregistration"
                );
                assert_eq!(jj::operation_id(repository.path()).await?, native);
            }
            let resolved = if stage == 1 || stage == 2 {
                storage
                    .resume_node_deletion(
                        operation.id,
                        Some(RecoveryConfirmation::ProcessesStopped),
                    )
                    .await?
            } else {
                storage
                    .recover_dependency_operation(
                        operation.id,
                        Some(RecoveryConfirmation::ProcessesStopped),
                    )
                    .await?
            };
            assert_eq!(storage.node(workflow, node).await.is_err(), stage != 0);
            if stage == 0 || stage == 3 {
                assert_eq!(jj::operation_id(repository.path()).await?, native);
            }
            let end = jj::operation_id(repository.path()).await?;
            assert_eq!(
                storage
                    .resume_node_deletion(
                        operation.id,
                        Some(RecoveryConfirmation::ProcessesStopped)
                    )
                    .await?,
                resolved
            );
            assert_eq!(
                jj::operation_id(repository.path()).await?,
                end,
                "resolved retry never runs jj"
            );
            assert_eq!(
                std::fs::read_to_string(directory.join("dirty.txt"))?,
                "keep every byte\n"
            );
            assert_eq!(
                std::fs::read(directory.join(".jj/working_copy/checkout"))?,
                checkout
            );
            assert_eq!(
                std::fs::read(directory.join(".jj/working_copy/tree_state"))?,
                tree
            );
            assert!(
                storage
                    .history_status(repository.path())
                    .await?
                    .undo
                    .is_none()
            );
        }
        Ok(())
    })
}

#[test]
fn failed_checkpoint_write_is_inspectable_and_resumable_without_repeating_abandonment()
-> anyhow::Result<()> {
    run(async {
        let (repository, mut storage, workflow, node) = fixture().await?;
        let kept = tempfile::tempdir()?;
        let directory = kept.path().join("workspace");
        storage
            .create_node_workspace(workflow, node, &directory)
            .await?;
        let mut operation = storage.preflight_deletion(workflow, node).await?;
        storage.reserve_reparent(&mut operation).await?;
        let connection = storage.database.connect()?;
        connection
            .execute(
                "INSERT INTO dependency_operations(workflow_id,active,record) VALUES (?1,0,'{}')",
                [workflow],
            )
            .await?;
        connection.execute("CREATE UNIQUE INDEX fail_checkpoint ON dependency_operations((record NOT LIKE '%\"abandoned_operation\":null%'))", ()).await?;
        jj::abandon_change(
            repository.path(),
            &operation.source,
            &operation.native_operation,
        )
        .await?;
        assert!(
            storage
                .checkpoint_abandoned_deletion(&mut operation)
                .await
                .is_err()
        );
        assert!(
            operation
                .deletion_workspace
                .as_ref()
                .expect("workspace")
                .abandoned_operation
                .is_none()
        );
        assert_eq!(
            storage
                .inspect_dependency_recovery(operation.id)
                .await?
                .assessment,
            DependencyRecoveryAssessment::DeletionAbandoned
        );
        let native = jj::operation_id(repository.path()).await?;
        connection.execute("DROP INDEX fail_checkpoint", ()).await?;
        let completed = storage
            .resume_node_deletion(operation.id, Some(RecoveryConfirmation::ProcessesStopped))
            .await?;
        assert_eq!(
            completed
                .deletion_workspace
                .expect("workspace")
                .abandoned_operation
                .as_deref(),
            Some(native.as_str())
        );
        assert!(directory.is_dir());
        Ok(())
    })
}

#[test]
fn intervening_native_writes_keep_partial_deletion_reserved() -> anyhow::Result<()> {
    run(async {
        let (repository, mut storage, workflow, node) = fixture().await?;
        let kept = tempfile::tempdir()?;
        storage
            .create_node_workspace(workflow, node, &kept.path().join("workspace"))
            .await?;
        let mut operation = storage.preflight_deletion(workflow, node).await?;
        storage.reserve_reparent(&mut operation).await?;
        jj::abandon_change(
            repository.path(),
            &operation.source,
            &operation.native_operation,
        )
        .await?;
        storage
            .checkpoint_abandoned_deletion(&mut operation)
            .await?;
        jj::create_change(repository.path(), &operation.destination).await?;
        let native = jj::operation_id(repository.path()).await?;
        assert!(matches!(
            storage
                .inspect_dependency_recovery(operation.id)
                .await?
                .assessment,
            DependencyRecoveryAssessment::Blocked { .. }
        ));
        assert!(
            storage
                .resume_node_deletion(operation.id, Some(RecoveryConfirmation::ProcessesStopped))
                .await
                .is_err()
        );
        assert!(
            storage
                .recover_dependency_operation(
                    operation.id,
                    Some(RecoveryConfirmation::ProcessesStopped)
                )
                .await
                .is_err()
        );
        assert_eq!(jj::operation_id(repository.path()).await?, native);
        assert_eq!(
            storage
                .workflow_graph(workflow)
                .await?
                .unresolved_dependency_operation,
            Some(operation.id)
        );
        Ok(())
    })
}
