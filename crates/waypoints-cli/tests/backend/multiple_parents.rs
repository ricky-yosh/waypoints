use super::*;

#[test]
fn multi_parent_cli_metadata_and_rpc_are_atomic_strict_and_restartable_without_jj()
-> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    jj(fixture.directory.path(), &["git", "init", "project"])?;
    let project = fixture.directory.path().join("project");
    let project = project.to_str().expect("project");
    fixture.success(&[
        "workflow",
        "create",
        "--project",
        project,
        "--name",
        "Parallel",
        "--base-revision",
        "root()",
    ])?;
    for prompt in ["API", "UI", "Database"] {
        fixture.success(&[
            "node",
            "create",
            "--workflow",
            "1",
            "--prompt",
            prompt,
            "--from-base",
        ])?;
    }
    let before = fixture.success(&["workflow", "graph", "--workflow", "1"])?;
    let history = fixture.success(&["history", "status", "--project", project])?;
    for params in [
        json!({"parents":null}),
        json!({"parents":{}}),
        json!({"parents":[{"kind":"node","node_id":"1"}]}),
        json!({"parent":{"kind":"node","node_id":1},"parents":[{"kind":"node","node_id":2}]}),
        json!({"parents":[{"kind":"node","node_id":1,"force":true}]}),
    ] {
        let mut params = params;
        params["workflow_id"] = json!(1);
        params["prompt"] = json!("Invalid");
        let response = fixture.rpc(
            format!(
                "{}\n",
                json!({"jsonrpc":"2.0","id":1,"method":"node.create","params":params})
            )
            .as_bytes(),
        )?;
        assert_eq!(response["error"]["code"], -32602, "{response}");
    }
    for parents in [
        json!([]),
        json!([{"kind":"node","node_id":1},{"kind":"node","node_id":1}]),
        json!([{"kind":"workflow_base"},{"kind":"node","node_id":1}]),
    ] {
        let response = fixture.rpc(format!("{}\n", json!({"jsonrpc":"2.0","id":1,"method":"node.create","params":{"workflow_id":1,"prompt":"Invalid","parents":parents}})).as_bytes())?;
        assert_eq!(response["error"]["code"], -32000, "{response}");
    }
    assert_eq!(
        fixture.success(&["workflow", "graph", "--workflow", "1"])?,
        before
    );
    assert_eq!(
        fixture.success(&["history", "status", "--project", project])?,
        history
    );
    fixture.stop()?;
    fixture.json_output(
        fixture
            .command()
            .env("PATH", "")
            .args([
                "node",
                "create",
                "--workflow",
                "1",
                "--prompt",
                "Integrate",
                "--parent",
                "3",
                "--parent",
                "1,2",
            ])
            .output()?,
        "multi-parent metadata without jj",
    )?;
    let graph = fixture.success(&["workflow", "graph", "--workflow", "1"])?;
    assert_eq!(graph["connections"].as_array().expect("edges").len(), 3);
    let history = fixture.success(&["history", "status", "--project", project])?;
    let edit = history["undo"]["id"].to_string();
    fixture.success(&["history", "undo", "--project", project, "--expected", &edit])?;
    assert_eq!(
        fixture.success(&["workflow", "graph", "--workflow", "1"])?,
        before
    );
    fixture.stop()?;
    fixture.json_output(
        fixture
            .command()
            .env("PATH", "")
            .args(["history", "redo", "--project", project, "--expected", &edit])
            .output()?,
        "multi-parent redo without jj",
    )?;
    assert_eq!(
        fixture.success(&["workflow", "graph", "--workflow", "1"])?,
        graph
    );
    Ok(())
}

#[test]
fn three_parent_creation_refuses_busy_input_then_supports_workspace_agent_snapshot_and_review()
-> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    jj(fixture.directory.path(), &["git", "init", "project"])?;
    let project = fixture.directory.path().join("project");
    let project_path = project.to_str().expect("project");
    fixture.success(&[
        "workflow",
        "create",
        "--project",
        project_path,
        "--name",
        "Parallel",
        "--base-revision",
        "root()",
    ])?;
    let mut commits = Vec::new();
    for (index, name) in ["api", "ui", "database"].iter().enumerate() {
        jj(&project, &["new", "root()"])?;
        fs::write(project.join(format!("{name}.txt")), format!("{name}\n"))?;
        jj(&project, &["describe", "-m", name])?;
        let change = jj(
            &project,
            &["log", "--no-graph", "-r", "@", "-T", "change_id"],
        )?;
        commits.push(jj(
            &project,
            &["log", "--no-graph", "-r", "@", "-T", "commit_id"],
        )?);
        fixture.success(&[
            "node",
            "create",
            "--workflow",
            "1",
            "--prompt",
            "permission",
            "--from-base",
        ])?;
        fixture.success(&[
            "node",
            "associate",
            "--workflow",
            "1",
            "--node",
            &(index + 1).to_string(),
            "--change",
            &change,
        ])?;
    }
    fixture.success(&[
        "node",
        "create",
        "--workflow",
        "1",
        "--prompt",
        "remember integration",
        "--parent",
        "1",
        "--parent",
        "2",
        "--parent",
        "3",
    ])?;
    let source_workspace = fixture.directory.path().join("source-workspace");
    fixture.success(&[
        "node",
        "workspace",
        "create",
        "--workflow",
        "1",
        "--node",
        "1",
        "--directory",
        source_workspace.to_str().expect("workspace"),
    ])?;
    let agent = fixture.directory.path().join("fake-acp");
    fs::write(&agent, include_str!("../fixtures/acp_agent.py"))?;
    fs::set_permissions(&agent, fs::Permissions::from_mode(0o700))?;
    let started = start_run(&fixture, &agent)?;
    let source_run = started["run"]["id"].to_string();
    wait_run(&fixture, &source_run, "awaiting_permission")?;
    assert!(
        !fixture
            .command()
            .args(["node", "create-change", "--workflow", "1", "--node", "4"])
            .output()?
            .status
            .success()
    );
    fixture.success(&["run", "cancel", "--run", &source_run])?;
    wait_run(&fixture, &source_run, "cancelled")?;
    let created = fixture.success(&["node", "create-change", "--workflow", "1", "--node", "4"])?;
    assert_eq!(created["state"]["revision"]["conflicted"], false);
    let workspace = fixture.directory.path().join("integration-workspace");
    fixture.success(&[
        "node",
        "workspace",
        "create",
        "--workflow",
        "1",
        "--node",
        "4",
        "--directory",
        workspace.to_str().expect("workspace"),
    ])?;
    for name in ["api", "ui", "database"] {
        assert_eq!(
            fs::read_to_string(workspace.join(format!("{name}.txt")))?,
            format!("{name}\n")
        );
    }
    assert!(
        !workspace.join("captured-prompt.txt").exists(),
        "cancelled upstream edits must not be snapshotted into merge"
    );
    let started = fixture.success(&[
        "run",
        "start",
        "--workflow",
        "1",
        "--node",
        "4",
        "--executable",
        agent.to_str().expect("agent"),
    ])?;
    let id = started["run"]["id"].to_string();
    let completed = wait_run(&fixture, &id, "completed")?;
    assert_eq!(
        completed["run"]["upstream_inputs"]
            .as_array()
            .expect("inputs")
            .len(),
        3
    );
    assert_eq!(completed["run"]["snapshot"]["status"], "saved");
    fixture.stop()?;
    let review = fixture.success(&["run", "review", "--run", &id])?;
    let mut actual: Vec<String> = serde_json::from_value(review["parent_commit_ids"].clone())?;
    actual.sort();
    commits.sort();
    assert_eq!(actual, commits);
    assert_eq!(
        fs::read_to_string(workspace.join("hello.txt"))?,
        "Hello from the first turn.\n"
    );
    Ok(())
}
