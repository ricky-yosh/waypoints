use super::*;

#[test]
fn rename_cli_and_rpc_preserve_base_and_round_trip_without_jj() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    jj(fixture.directory.path(), &["git", "init", "project"])?;
    let project = fixture.directory.path().join("project");
    let project = project.to_str().expect("project path");
    let original = fixture.success(&[
        "workflow",
        "create",
        "--project",
        project,
        "--name",
        "Original",
        "--base-revision",
        "root()",
    ])?;
    fixture.success(&[
        "node",
        "create",
        "--workflow",
        "1",
        "--prompt",
        "Root",
        "--from-base",
    ])?;
    let before = fixture.success(&["workflow", "graph", "--workflow", "1"])?;
    fixture.stop()?;
    let name = "  Renamed 'workflow' 日本語  ";
    let renamed = fixture.json_output(
        fixture
            .command()
            .env("PATH", "")
            .args(["workflow", "rename", "--workflow", "1", "--name", name])
            .output()?,
        "rename without jj",
    )?;
    let mut expected = original.clone();
    expected["name"] = json!(name);
    assert_eq!(renamed, expected);
    let pending = fixture.success(&["history", "status", "--project", project])?;
    let edit = pending["undo"]["id"].to_string();
    assert_eq!(pending["undo"]["label"], "Rename workflow");
    let mut graph = before.clone();
    graph["workflow"] = renamed.clone();
    assert_eq!(
        fixture.success(&["workflow", "graph", "--workflow", "1"])?,
        graph
    );
    assert_eq!(
        fixture.success(&["workflow", "list", "--project", project])?,
        json!([renamed.clone()])
    );

    for params in [
        json!({"workflow_id":1}),
        json!({"workflow_id":1,"name":null}),
        json!({"workflow_id":"1","name":"Bad"}),
        json!({"workflow_id":1,"name":12}),
        json!({"workflow_id":1,"name":"Bad","force":true}),
    ] {
        let response = fixture.rpc(
            format!(
                "{}\n",
                json!({
                    "jsonrpc":"2.0","id":1,"method":"workflow.rename","params":params
                })
            )
            .as_bytes(),
        )?;
        assert_eq!(response["error"]["code"], -32602, "{response}");
    }
    for params in [
        json!({"workflow_id":1,"name":" \n\t"}),
        json!({"workflow_id":999,"name":"Missing"}),
    ] {
        let response = fixture.rpc(
            format!(
                "{}\n",
                json!({
                    "jsonrpc":"2.0","id":1,"method":"workflow.rename","params":params
                })
            )
            .as_bytes(),
        )?;
        assert_eq!(response["error"]["code"], -32000, "{response}");
    }
    assert_eq!(
        fixture.success(&["history", "status", "--project", project])?,
        pending
    );
    fixture.stop()?;
    let loaded = fixture.json_output(
        fixture
            .command()
            .env("PATH", "")
            .args(["workflow", "get", "--workflow", "1"])
            .output()?,
        "restart without jj",
    )?;
    assert_eq!(loaded, renamed);
    fixture.success(&["history", "undo", "--project", project, "--expected", &edit])?;
    assert_eq!(
        fixture.success(&["workflow", "graph", "--workflow", "1"])?,
        before
    );
    // An identical name must leave the saved redo action intact.
    fixture.success(&[
        "workflow",
        "rename",
        "--workflow",
        "1",
        "--name",
        "Original",
    ])?;
    assert_eq!(
        fixture.success(&["history", "status", "--project", project])?["redo"]["id"].to_string(),
        edit
    );
    fixture.stop()?;
    fixture.json_output(
        fixture
            .command()
            .env("PATH", "")
            .args(["history", "redo", "--project", project, "--expected", &edit])
            .output()?,
        "redo after restart without jj",
    )?;
    assert_eq!(
        fixture.success(&["workflow", "graph", "--workflow", "1"])?,
        graph
    );
    Ok(())
}

#[test]
fn rename_and_undo_leave_active_runs_workspaces_and_native_state_untouched() -> anyhow::Result<()> {
    let (fixture, agent, workspace) = run_fixture("permission")?;
    let started = start_run(&fixture, &agent)?;
    let id = started["run"]["id"].to_string();
    let waiting = wait_run(&fixture, &id, "awaiting_permission")?;
    let project = fixture.directory.path().join("project");
    let project = project.to_str().expect("project path");
    let before = fixture.success(&["workflow", "graph", "--workflow", "1"])?;
    let allocation = fixture.success(&[
        "node",
        "workspace",
        "status",
        "--workflow",
        "1",
        "--node",
        "1",
    ])?;
    fs::write(workspace.join("manual.txt"), "must not be snapshotted\n")?;
    let head = jj(
        &workspace,
        &[
            "--ignore-working-copy",
            "--at-op=@",
            "op",
            "log",
            "--limit",
            "1",
            "-T",
            "id",
        ],
    )?;
    fixture.success(&[
        "workflow",
        "rename",
        "--workflow",
        "1",
        "--name",
        "Active workflow",
    ])?;
    let history = fixture.success(&["history", "status", "--project", project])?;
    let edit = history["undo"]["id"].to_string();
    fixture.success(&["history", "undo", "--project", project, "--expected", &edit])?;
    assert_eq!(
        fixture.success(&["workflow", "graph", "--workflow", "1"])?,
        before
    );
    fixture.success(&["history", "redo", "--project", project, "--expected", &edit])?;
    assert_eq!(
        fixture.success(&["run", "status", "--run", &id])?["run"],
        waiting["run"]
    );
    assert_eq!(
        fixture.success(&[
            "node",
            "workspace",
            "status",
            "--workflow",
            "1",
            "--node",
            "1"
        ])?,
        allocation
    );
    assert_eq!(
        fs::read_to_string(workspace.join("manual.txt"))?,
        "must not be snapshotted\n"
    );
    assert_eq!(
        jj(
            &workspace,
            &[
                "--ignore-working-copy",
                "--at-op=@",
                "op",
                "log",
                "--limit",
                "1",
                "-T",
                "id"
            ]
        )?,
        head
    );
    fixture.success(&["run", "cancel", "--run", &id])?;
    wait_run(&fixture, &id, "cancelled")?;
    Ok(())
}
