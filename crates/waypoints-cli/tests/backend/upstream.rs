use super::*;

#[test]
fn hint_is_readonly_and_does_not_change_agent_prompts() -> anyhow::Result<()> {
    let (fixture, agent, _workspace) = run_fixture("unused")?;
    let project = fixture.directory.path().join("project");
    fixture.success(&[
        "workflow",
        "create",
        "--project",
        project.to_str().expect("project"),
        "--name",
        "Based",
        "--base-revision",
        "root()",
    ])?;
    fixture.success(&[
        "node",
        "create",
        "--workflow",
        "2",
        "--prompt",
        "A",
        "--from-base",
    ])?;
    let upstream = fixture.success(&["node", "create-change", "--workflow", "2", "--node", "2"])?;
    fixture.success(&[
        "node",
        "create",
        "--workflow",
        "2",
        "--prompt",
        "complete",
        "--parent",
        "2",
    ])?;
    fixture.success(&["node", "create-change", "--workflow", "2", "--node", "3"])?;
    let workspace = fixture.directory.path().join("dependent-workspace");
    fixture.success(&[
        "node",
        "workspace",
        "create",
        "--workflow",
        "2",
        "--node",
        "3",
        "--directory",
        workspace.to_str().expect("workspace"),
    ])?;
    let started = fixture.success(&[
        "run",
        "start",
        "--workflow",
        "2",
        "--node",
        "3",
        "--executable",
        agent.to_str().expect("agent"),
    ])?;
    let id = started["run"]["id"].to_string();
    let completed = wait_run(&fixture, &id, "completed")?;
    assert!(completed["run"]["upstream_inputs"].is_array());
    assert_eq!(
        fs::read_to_string(workspace.join("captured-prompt.txt"))?,
        "complete"
    );
    assert_eq!(
        fixture.success(&["run", "inputs", "--run", &id])?["status"],
        "unchanged"
    );
    jj(
        &project,
        &[
            "--ignore-working-copy",
            "describe",
            "-r",
            upstream["association"]["change_id"]
                .as_str()
                .expect("upstream"),
            "-m",
            "Changed upstream",
        ],
    )?;
    let before = recorded_commit(&workspace)?;
    fs::write(workspace.join("dirty.txt"), "not recorded\n")?;
    assert_eq!(
        fixture.success(&["run", "inputs", "--run", &id])?["status"],
        "changed"
    );
    assert_eq!(
        fixture.success(&["run", "status", "--run", &id])?,
        completed
    );
    let human = Command::new(env!("CARGO_BIN_EXE_waypoints"))
        .arg("--database")
        .arg(&fixture.database)
        .arg("--socket")
        .arg(&fixture.socket)
        .args(["run", "status", "--run", &id])
        .output()?;
    assert!(human.status.success());
    assert!(String::from_utf8(human.stdout)?.contains("changed since this run"));
    assert_eq!(recorded_commit(&workspace)?, before);
    assert_eq!(
        fs::read_to_string(workspace.join("dirty.txt"))?,
        "not recorded\n"
    );
    fixture.stop()?;
    assert_eq!(
        fixture.success(&["run", "inputs", "--run", &id])?["status"],
        "changed"
    );
    let mut bytes = serde_json::to_vec(&json!({
        "jsonrpc": "2.0", "id": 1, "method": "run.inputs",
        "params": {"run_id": id.parse::<i64>()?, "acknowledge": true}
    }))?;
    bytes.push(b'\n');
    assert_eq!(fixture.rpc(&bytes)?["error"]["code"], -32602);
    Ok(())
}
