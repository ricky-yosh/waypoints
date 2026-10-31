use std::fs;
use std::future::Future;

use waypoints_core::storage::{Connection, Node, Storage, StorageError, WorkflowGraph};

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

#[test]
fn graphs_preserve_prompts_ordering_and_connections_after_restart() -> anyhow::Result<()> {
    run_test(async {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("workflows.db");
        let mut storage = Storage::open(&path).await?;
        let workflow = storage.create_workflow(directory.path(), "Graph").await?;
        let empty = storage.workflow_graph(workflow.id).await?;
        assert_eq!(empty.workflow, workflow);
        assert!(empty.nodes.is_empty());
        assert!(empty.connections.is_empty());

        let prompt = "\0 \nIt's a prompt'); DROP TABLE nodes; --\t\u{2003}";
        let first = storage.create_node(workflow.id, prompt).await?;
        assert_eq!(first.prompt, prompt);
        let second = storage.create_node(workflow.id, "Second").await?;
        let third = storage.create_node(workflow.id, "Third").await?;
        let fourth = storage.create_node(workflow.id, "Fourth").await?;
        // Insert out of order; a diamond exercises both fan-out and fan-in.
        let third_fourth = storage
            .connect_nodes(workflow.id, third.id, fourth.id)
            .await?;
        let first_third = storage
            .connect_nodes(workflow.id, first.id, third.id)
            .await?;
        let second_fourth = storage
            .connect_nodes(workflow.id, second.id, fourth.id)
            .await?;
        let first_second = storage
            .connect_nodes(workflow.id, first.id, second.id)
            .await?;
        let updated_prompt = "\n Updated ' prompt \t";
        let second = storage
            .update_node(workflow.id, second.id, updated_prompt)
            .await?;
        assert_eq!(second.prompt, updated_prompt);
        assert_eq!(storage.node(workflow.id, second.id).await?, second);
        let graph = storage.workflow_graph(workflow.id).await?;
        assert_eq!(graph.nodes, vec![first, second, third, fourth]);
        assert_eq!(
            graph.connections,
            vec![first_second, first_third, second_fourth, third_fourth]
        );
        assert_eq!(
            serde_json::from_str::<WorkflowGraph>(&serde_json::to_string(&graph)?)?,
            graph
        );
        assert_eq!(
            serde_json::from_str::<Node>(&serde_json::to_string(&graph.nodes[0])?)?,
            graph.nodes[0]
        );
        assert_eq!(
            serde_json::from_str::<Connection>(&serde_json::to_string(&first_second)?)?,
            first_second
        );
        drop(storage);

        let mut storage = Storage::open(&path).await?;
        assert_eq!(storage.workflow_graph(workflow.id).await?, graph);
        let extra = storage.create_node(workflow.id, "After restart").await?;
        assert!(extra.id > graph.nodes[3].id);
        storage
            .connect_nodes(workflow.id, graph.nodes[3].id, extra.id)
            .await?;
        assert!(
            storage
                .connect_nodes(workflow.id, extra.id, graph.nodes[0].id)
                .await
                .is_err()
        );
        Ok(())
    })
}

#[test]
fn invalid_graph_operations_preserve_both_workflows() -> anyhow::Result<()> {
    run_test(async {
        let directory = tempfile::tempdir()?;
        let mut storage = Storage::open(&directory.path().join("workflows.db")).await?;
        let first = storage.create_workflow(directory.path(), "First").await?;
        let second = storage.create_workflow(directory.path(), "Second").await?;
        let a = storage.create_node(first.id, "A").await?;
        let b = storage.create_node(first.id, "B").await?;
        let c = storage.create_node(first.id, "C").await?;
        let d = storage.create_node(second.id, "D").await?;
        let e = storage.create_node(second.id, "E").await?;
        storage.connect_nodes(first.id, a.id, b.id).await?;
        storage.connect_nodes(first.id, b.id, c.id).await?;
        storage.connect_nodes(second.id, d.id, e.id).await?;
        let before_first = storage.workflow_graph(first.id).await?;
        let before_second = storage.workflow_graph(second.id).await?;

        for prompt in ["", " ", "\t\r\n", "\u{2003}\u{00a0}"] {
            assert!(
                storage.create_node(first.id, prompt).await.is_err(),
                "{prompt:?}"
            );
            assert!(
                storage.update_node(first.id, a.id, prompt).await.is_err(),
                "{prompt:?}"
            );
            assert_eq!(storage.workflow_graph(first.id).await?, before_first);
        }
        for missing in [-1, 0, i64::MAX] {
            assert!(storage.create_node(missing, "Prompt").await.is_err());
            assert!(storage.update_node(missing, a.id, "Changed").await.is_err());
            assert!(storage.node(missing, a.id).await.is_err());
            assert!(storage.workflow_graph(missing).await.is_err());
            assert!(
                storage
                    .update_node(first.id, missing, "Changed")
                    .await
                    .is_err()
            );
            assert!(storage.node(first.id, missing).await.is_err());
            assert!(
                storage
                    .connect_nodes(first.id, missing, a.id)
                    .await
                    .is_err()
            );
            assert!(
                storage
                    .connect_nodes(first.id, a.id, missing)
                    .await
                    .is_err()
            );
            assert!(storage.connect_nodes(missing, a.id, b.id).await.is_err());
            assert_eq!(storage.workflow_graph(first.id).await?, before_first);
            assert_eq!(storage.workflow_graph(second.id).await?, before_second);
        }
        assert!(storage.node(first.id, d.id).await.is_err());
        assert!(storage.node(second.id, a.id).await.is_err());
        assert!(
            storage
                .update_node(first.id, d.id, "Changed")
                .await
                .is_err()
        );
        assert!(
            storage
                .update_node(second.id, a.id, "Changed")
                .await
                .is_err()
        );
        for (workflow_id, source, target) in [
            (first.id, a.id, a.id), // self-link
            (first.id, a.id, b.id), // duplicate
            (first.id, b.id, a.id), // direct cycle
            (first.id, c.id, a.id), // transitive cycle
            (first.id, a.id, d.id), // foreign target
            (first.id, d.id, a.id), // foreign source
            (first.id, d.id, e.id), // both endpoints foreign
            (second.id, a.id, b.id),
        ] {
            assert!(
                storage
                    .connect_nodes(workflow_id, source, target)
                    .await
                    .is_err(),
                "workflow {workflow_id}: {source} -> {target}"
            );
            assert_eq!(storage.workflow_graph(first.id).await?, before_first);
            assert_eq!(storage.workflow_graph(second.id).await?, before_second);
        }
        let replacement = storage.update_node(second.id, d.id, "Updated D").await?;
        assert_eq!(storage.node(second.id, d.id).await?, replacement);
        assert_eq!(storage.workflow_graph(first.id).await?, before_first);
        Ok(())
    })
}

#[test]
fn workflows_only_database_is_upgraded_without_losing_data() -> anyhow::Result<()> {
    run_test(async {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("workflows.db");
        let project = fs::canonicalize(directory.path())?;
        {
            let database =
                turso::Builder::new_local(path.to_str().expect("temporary path is UTF-8"))
                    .build()
                    .await?;
            let connection = database.connect()?;
            connection
                .execute(
                    "CREATE TABLE workflows (
                    id INTEGER PRIMARY KEY,
                    project_path TEXT NOT NULL,
                    name TEXT NOT NULL CHECK (length(trim(name)) > 0)
                )",
                    (),
                )
                .await?;
            connection
                .execute(
                    "INSERT INTO workflows (id, project_path, name) VALUES (42, ?1, ?2)",
                    [
                        project.to_str().expect("temporary path is UTF-8"),
                        " Old workflow ",
                    ],
                )
                .await?;
        }
        let mut storage = Storage::open(&path).await?;
        let workflows = storage.list_workflows(&project).await?;
        assert_eq!(workflows.len(), 1);
        assert_eq!(workflows[0].id, 42);
        assert_eq!(workflows[0].name, " Old workflow ");
        let graph = storage.workflow_graph(42).await?;
        assert_eq!(graph.workflow, workflows[0]);
        assert!(graph.nodes.is_empty());
        assert!(graph.connections.is_empty());
        let a = storage.create_node(42, "A").await?;
        let b = storage.create_node(42, "B").await?;
        storage.connect_nodes(42, a.id, b.id).await?;
        let graph = storage.workflow_graph(42).await?;
        let new_workflow = storage.create_workflow(&project, "New workflow").await?;
        assert!(new_workflow.id > 42);
        drop(storage);
        let storage = Storage::open(&path).await?;
        assert_eq!(storage.workflow_graph(42).await?, graph);
        assert_eq!(storage.list_workflows(&project).await?.len(), 2);
        Ok(())
    })
}
