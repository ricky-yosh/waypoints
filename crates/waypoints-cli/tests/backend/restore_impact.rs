use super::*;

fn native_operation(project: &Path) -> anyhow::Result<String> {
    jj(
        project,
        &[
            "--ignore-working-copy",
            "--at-op=@",
            "op",
            "log",
            "--no-graph",
            "--limit",
            "1",
            "-T",
            "id",
        ],
    )
}

#[test]
fn restore_impact_cli_and_rpc_are_readonly_strict_and_restartable() -> anyhow::Result<()> {
    let (fixture, agent, workspace) = run_fixture("remember upstream")?;
    let started = start_run(&fixture, &agent)?;
    let run_id = started["run"]["id"].to_string();
    let completed = wait_run(&fixture, &run_id, "completed")?;
    let selected = completed["run"]["snapshot"]["revision"]["commit_id"]
        .as_str()
        .unwrap();
    fs::write(workspace.join("manual.txt"), "recorded\n")?;
    jj(&workspace, &["describe", "-m", "manual"])?;
    let current = recorded_commit(&workspace)?;
    let child = fixture.success(&[
        "node",
        "create",
        "--workflow",
        "1",
        "--prompt",
        "Downstream task",
    ])?;
    fixture.success(&[
        "connection",
        "create",
        "--workflow",
        "1",
        "--source",
        "1",
        "--target",
        &child["id"].to_string(),
    ])?;
    fs::write(workspace.join("manual.txt"), "dirty\n")?;
    let project = fixture.directory.path().join("project");
    let operation = native_operation(&project)?;
    let graph = fixture.success(&["workflow", "graph", "--workflow", "1"])?;
    let request = [
        "node",
        "restore-impact",
        "--workflow",
        "1",
        "--node",
        "1",
        "--commit",
        selected,
    ];
    let impact = fixture.success(&request)?;
    assert_eq!(impact["native_operation"], operation);
    assert_eq!(impact["current"]["commit_id"], current);
    assert_eq!(impact["selected"]["commit_id"], selected);
    assert_eq!(impact["changes"].as_array().unwrap().len(), 1);
    assert_eq!(impact["tasks"].as_array().unwrap().len(), 2);
    let prompt = impact["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["node"]["id"] == child["id"])
        .unwrap();
    assert_eq!(prompt["native_affected"], false);
    assert_eq!(
        impact["sessions"][0]["run_ids"],
        json!([completed["run"]["id"]])
    );
    assert_eq!(impact["sessions"][0]["source_node"], true);
    assert!(
        impact["observed_blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b.as_str().unwrap().contains("downstream restore"))
    );
    fixture.stop()?;
    assert_eq!(fixture.success(&request)?, impact);
    let human = Command::new(env!("CARGO_BIN_EXE_waypoints"))
        .arg("--database")
        .arg(&fixture.database)
        .arg("--socket")
        .arg(&fixture.socket)
        .args(request)
        .output()?;
    assert!(
        human.status.success(),
        "{}",
        String::from_utf8_lossy(&human.stderr)
    );
    assert_eq!(serde_json::from_slice::<Value>(&human.stdout)?, impact);
    assert!(
        human.stdout.contains(&b'\n'),
        "non-JSON flag output should be pretty JSON"
    );
    for (params, code) in [
        (json!({"workflow_id":1,"node_id":1,"commit_id":"@"}), -32000),
        (
            json!({"workflow_id":1,"node_id":1,"commit_id":selected[..12]}),
            -32000,
        ),
        (
            json!({"workflow_id":1,"node_id":1,"commit_id":selected,"apply":true}),
            -32602,
        ),
        (
            json!({"workflow_id":1,"node_id":1,"commit_id":selected,"force":true}),
            -32602,
        ),
        (json!({"workflow_id":1,"node_id":1,"commit_id":42}), -32602),
        (json!({"workflow_id":1,"node_id":1}), -32602),
    ] {
        let response = fixture.rpc(
            format!(
                "{}\n",
                json!({
                    "jsonrpc":"2.0", "id":1, "method":"node.restore_impact", "params":params,
                })
            )
            .as_bytes(),
        )?;
        assert_eq!(response["error"]["code"], code, "{response}");
    }
    assert_eq!(native_operation(&project)?, operation);
    assert_eq!(recorded_commit(&workspace)?, current);
    assert_eq!(fs::read_to_string(workspace.join("manual.txt"))?, "dirty\n");
    assert_eq!(
        fixture.success(&["workflow", "graph", "--workflow", "1"])?,
        graph
    );
    assert_eq!(
        fixture.success(&["run", "status", "--run", &run_id])?,
        completed
    );
    assert_eq!(
        fixture.success(&["node", "restores", "--workflow", "1", "--node", "1"])?,
        json!([])
    );
    assert_eq!(
        agent_requests(&agent)?
            .iter()
            .filter(|r| r["method"] == "session/new")
            .count(),
        1
    );
    Ok(())
}

#[test]
fn restore_impact_rejects_concurrent_native_changes_instead_of_returning_a_mixed_report()
-> anyhow::Result<()> {
    let (fixture, _agent, workspace) = run_fixture("unused")?;
    let selected = recorded_commit(&workspace)?;
    let project = fixture.directory.path().join("project");
    let operation = native_operation(&project)?;
    fixture.stop()?;
    let native = Command::new("/bin/sh")
        .args(["-c", "command -v jj"])
        .output()?;
    assert!(native.status.success());
    let native = String::from_utf8(native.stdout)?.trim().to_owned();
    let bin = fixture.directory.path().join("impact-bin");
    fs::create_dir(&bin)?;
    let marker = fixture.directory.path().join("external-write");
    let wrapper = bin.join("jj");
    // Only the backend inherits this wrapper. The mutation simulates an outside
    // writer after ancestry was read; all subsequent preview queries stay pinned.
    fs::write(
        &wrapper,
        format!(
            r#"#!/bin/sh
native='{native}'
case " $* " in
  *" workspace list "*)
    if [ ! -e '{marker}' ]; then
      touch '{marker}'
      "$native" --ignore-working-copy --at-op=@ describe -r '{selected}' -m 'external rewrite' >/dev/null 2>&1 || exit 42
    fi
    ;;
esac
exec "$native" "$@"
"#,
            marker = marker.display()
        ),
    )?;
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700))?;
    let path = format!("{}:{}", bin.display(), std::env::var("PATH")?);
    let response = fixture
        .command()
        .env("PATH", path)
        .args([
            "node",
            "restore-impact",
            "--workflow",
            "1",
            "--node",
            "1",
            "--commit",
            &selected,
        ])
        .output()?;
    assert!(!response.status.success());
    assert!(
        String::from_utf8_lossy(&response.stderr)
            .contains("repository changed during restore impact"),
        "{}",
        String::from_utf8_lossy(&response.stderr)
    );
    assert!(marker.exists());
    assert_ne!(native_operation(&project)?, operation);
    let parents: Value = serde_json::from_str(&jj(
        &project,
        &[
            "--ignore-working-copy",
            "--at-op=@",
            "op",
            "log",
            "--no-graph",
            "--limit",
            "1",
            "-T",
            "json(parents.map(|p| p.id()))",
        ],
    )?)?;
    assert_eq!(
        parents,
        json!([operation]),
        "only the simulated outside writer may create an operation"
    );
    assert_eq!(
        fixture.success(&["node", "restores", "--workflow", "1", "--node", "1"])?,
        json!([])
    );
    let refreshed = fixture.success(&[
        "node",
        "restore-impact",
        "--workflow",
        "1",
        "--node",
        "1",
        "--commit",
        &selected,
    ])?;
    assert_ne!(refreshed["native_operation"], operation);
    assert_eq!(refreshed["workspaces"][0]["checkout"]["status"], "stale");
    Ok(())
}
