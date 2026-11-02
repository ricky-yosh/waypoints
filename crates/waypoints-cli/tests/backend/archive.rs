use super::*;

#[test]
fn archive_cli_rpc_lists_and_restart_without_jj() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let project = fixture.directory.path().to_str().expect("project path");
    let original =
        fixture.success(&["workflow", "create", "--project", project, "--name", "Keep"])?;
    fixture.success(&["node", "create", "--workflow", "1", "--prompt", "Reference"])?;
    let graph = fixture.success(&["workflow", "graph", "--workflow", "1"])?;
    let history = fixture.success(&["history", "status", "--project", project])?;
    fixture.stop()?;
    let archived = fixture.json_output(
        fixture
            .command()
            .env("PATH", "")
            .args(["workflow", "archive", "--workflow", "1"])
            .output()?,
        "archive without jj",
    )?;
    let mut expected = original.clone();
    expected["archived"] = json!(true);
    assert_eq!(archived, expected);
    assert_eq!(
        fixture.success(&["workflow", "archive", "--workflow", "1"])?,
        archived
    );
    assert_eq!(
        fixture.success(&["workflow", "list", "--project", project])?,
        json!([])
    );
    for flag in ["--archived", "--all"] {
        assert_eq!(
            fixture.success(&["workflow", "list", "--project", project, flag])?,
            json!([archived.clone()])
        );
    }
    let mut archived_graph = graph.clone();
    archived_graph["workflow"] = archived.clone();
    assert_eq!(
        fixture.success(&["workflow", "graph", "--workflow", "1"])?,
        archived_graph
    );
    assert_eq!(
        fixture.success(&["history", "status", "--project", project])?,
        history
    );
    // Old clients can omit the filter; strict validation still rejects unknown fields.
    let response = fixture.rpc(
        format!(
            "{}\n",
            json!({
                "jsonrpc":"2.0","id":1,"method":"workflow.list","params":{"project_path":project}
            })
        )
        .as_bytes(),
    )?;
    assert_eq!(response["result"], json!([]));
    for (method, params, code) in [
        ("workflow.archive", json!({"workflow_id":999}), -32000),
        ("workflow.unarchive", json!({"workflow_id":999}), -32000),
        (
            "workflow.archive",
            json!({"workflow_id":1,"force":true}),
            -32602,
        ),
        ("workflow.unarchive", json!({"workflow_id":"1"}), -32602),
        (
            "workflow.list",
            json!({"project_path":project,"filter":"invalid"}),
            -32602,
        ),
        (
            "workflow.list",
            json!({"project_path":project,"archived":true}),
            -32602,
        ),
    ] {
        let response = fixture.rpc(
            format!(
                "{}\n",
                json!({
                    "jsonrpc":"2.0","id":1,"method":method,"params":params
                })
            )
            .as_bytes(),
        )?;
        assert_eq!(response["error"]["code"], code, "{response}");
    }
    assert!(
        !fixture
            .command()
            .args([
                "workflow",
                "list",
                "--project",
                project,
                "--archived",
                "--all"
            ])
            .output()?
            .status
            .success()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_waypoints"))
        .arg("--database")
        .arg(&fixture.database)
        .arg("--socket")
        .arg(&fixture.socket)
        .args(["workflow", "list", "--project", project, "--all"])
        .output()?;
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout)?, "1\tKeep [archived]\n");
    fixture.stop()?;
    assert_eq!(
        fixture.success(&["workflow", "get", "--workflow", "1"])?,
        archived
    );
    let response = fixture.rpc(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"workflow.unarchive\",\"params\":{\"workflow_id\":1}}\n")?;
    assert_eq!(response["result"], original);
    assert_eq!(
        fixture.success(&["workflow", "unarchive", "--workflow", "1"])?,
        original
    );
    assert_eq!(
        fixture.success(&["workflow", "graph", "--workflow", "1"])?,
        graph
    );
    assert_eq!(
        fixture.success(&["workflow", "list", "--project", project])?,
        json!([original])
    );
    fixture.stop()?;
    assert_eq!(
        fixture.success(&["workflow", "list", "--project", project, "--archived"])?,
        json!([])
    );
    Ok(())
}

#[test]
fn archive_does_not_stop_runs_or_touch_native_workspaces() -> anyhow::Result<()> {
    let (fixture, agent, workspace) = run_fixture("permission")?;
    let started = start_run(&fixture, &agent)?;
    let id = started["run"]["id"].to_string();
    let waiting = wait_run(&fixture, &id, "awaiting_permission")?;
    fs::write(workspace.join("manual.txt"), "keep unsnapshotted edits\n")?;
    let native_head = || {
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
                "id",
            ],
        )
    };
    let head = native_head()?;
    let association = fixture.success(&["node", "refresh", "--workflow", "1", "--node", "1"])?;
    let allocation = fixture.success(&[
        "node",
        "workspace",
        "status",
        "--workflow",
        "1",
        "--node",
        "1",
    ])?;
    for command in ["archive", "unarchive"] {
        fixture.success(&["workflow", command, "--workflow", "1"])?;
        assert_eq!(
            fixture.success(&["run", "status", "--run", &id])?["run"],
            waiting["run"]
        );
        assert_eq!(
            fixture.success(&["node", "refresh", "--workflow", "1", "--node", "1"])?,
            association
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
            "keep unsnapshotted edits\n"
        );
        assert_eq!(native_head()?, head);
    }
    fixture.success(&["run", "cancel", "--run", &id])?;
    wait_run(&fixture, &id, "cancelled")?;
    Ok(())
}
