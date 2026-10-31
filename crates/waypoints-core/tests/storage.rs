use std::fs;
use std::future::Future;

use waypoints_core::storage::{Storage, StorageError};

// Use the existing current-thread runtime feature; Tokio test macros are not needed.
fn run_test(test: impl Future<Output = anyhow::Result<()>>) -> anyhow::Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .build()?
        .block_on(test)
}

#[test]
fn workflows_survive_reopening_database() -> anyhow::Result<()> {
    run_test(async {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("workflows.db");
        let project = directory.path().join("project");
        fs::create_dir(&project)?;

        let mut storage = Storage::open(&path).await?;
        let saved = storage.create_workflow(&project, "First workflow").await?;
        assert_eq!(saved.project_path, fs::canonicalize(&project)?);
        assert_eq!(saved.name, "First workflow");
        drop(storage);

        let storage = Storage::open(&path).await?;
        assert_eq!(storage.list_workflows(&project).await?, vec![saved]);
        Ok(())
    })
}

#[test]
fn workflows_are_project_scoped_ordered_and_parameterized() -> anyhow::Result<()> {
    run_test(async {
        let directory = tempfile::tempdir()?;
        let first_project = directory.path().join("first ' project");
        let second_project = directory.path().join("second");
        fs::create_dir(&first_project)?;
        fs::create_dir(&second_project)?;
        let mut storage = Storage::open(&directory.path().join("workflows.db")).await?;
        assert!(storage.list_workflows(&first_project).await?.is_empty());

        let name = "It's a workflow'); DROP TABLE workflows; --";
        let first = storage.create_workflow(&first_project, name).await?;
        let second = storage.create_workflow(&first_project, name).await?;
        let other = storage.create_workflow(&second_project, "Other").await?;
        assert_ne!(
            first.id, second.id,
            "duplicate names must have distinct IDs"
        );
        assert_eq!(first.name, name, "SQL-looking text must be preserved");
        assert_eq!(
            storage.list_workflows(&first_project.join(".")).await?,
            vec![first, second],
            "equivalent paths must resolve to the same ordered workflows"
        );
        assert_eq!(storage.list_workflows(&second_project).await?, vec![other]);
        Ok(())
    })
}

#[test]
fn invalid_inputs_do_not_create_workflows() -> anyhow::Result<()> {
    run_test(async {
        let directory = tempfile::tempdir()?;
        let database = directory.path().join("workflows.db");
        let mut storage = Storage::open(&database).await?;
        for name in ["", " ", "\t\n", "\u{2003}"] {
            let error = storage
                .create_workflow(directory.path(), name)
                .await
                .expect_err("blank names must be rejected");
            assert!(error.to_string().contains("name must not be blank"));
        }

        let missing = directory.path().join("missing");
        assert!(storage.create_workflow(&missing, "Invalid").await.is_err());
        assert!(storage.list_workflows(&missing).await.is_err());
        assert!(
            storage
                .create_workflow(&database, "Not a directory")
                .await
                .is_err()
        );
        assert!(storage.list_workflows(&database).await.is_err());
        assert!(storage.list_workflows(directory.path()).await?.is_empty());
        Ok(())
    })
}

#[test]
fn opening_in_a_missing_directory_returns_an_error() -> anyhow::Result<()> {
    run_test(async {
        let directory = tempfile::tempdir()?;
        assert!(
            Storage::open(&directory.path().join("missing/workflows.db"))
                .await
                .is_err()
        );
        Ok(())
    })
}

#[test]
fn storage_and_errors_are_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Storage>();
    assert_send_sync::<StorageError>();
}
