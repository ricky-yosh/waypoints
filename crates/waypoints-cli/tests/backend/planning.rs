use super::*;

#[test]
fn prompt_topology_cli_round_trips_without_jj_and_rejects_stale_undo() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    jj(fixture.directory.path(), &["git", "init", "project"])?;
    let project = fixture.directory.path().join("project");
    let project = project.to_str().expect("project path");
    fixture.success(&[
        "workflow",
        "create",
        "--project",
        project,
        "--name",
        "Planning",
        "--base-revision",
        "root()",
    ])?;
    for (prompt, parent) in [("A", None), ("B", Some("1")), ("C", Some("2")), ("D", None)] {
        let mut args = vec!["node", "create", "--workflow", "1", "--prompt", prompt];
        if let Some(parent) = parent {
            args.extend(["--parent", parent]);
        } else {
            args.push("--from-base");
        }
        fixture.success(&args)?;
    }
    let before = fixture.success(&["workflow", "graph", "--workflow", "1"])?;
    fixture.stop()?;
    fixture.json_output(
        fixture.list_command().env("PATH", "").output()?,
        "restart without jj",
    )?;
    let moved = fixture.success(&[
        "node",
        "move-prompt",
        "--workflow",
        "1",
        "--node",
        "2",
        "--parent",
        "4",
    ])?;
    assert!(
        moved["connections"]
            .as_array()
            .expect("edges")
            .iter()
            .any(|c| c["source_id"] == 4 && c["target_id"] == 2)
    );
    let move_history = fixture.success(&["history", "status", "--project", project])?;
    let move_id = move_history["undo"]["id"].to_string();
    let deleted = fixture.success(&["node", "delete-prompt", "--workflow", "1", "--node", "2"])?;
    assert!(
        deleted["connections"]
            .as_array()
            .expect("edges")
            .iter()
            .any(|c| c["source_id"] == 4 && c["target_id"] == 3)
    );
    assert!(
        !deleted["nodes"]
            .as_array()
            .expect("nodes")
            .iter()
            .any(|n| n["id"] == 2)
    );
    assert!(
        !fixture
            .command()
            .args([
                "history",
                "undo",
                "--project",
                project,
                "--expected",
                &move_id
            ])
            .output()?
            .status
            .success()
    );
    let history = fixture.success(&["history", "status", "--project", project])?;
    let delete_id = history["undo"]["id"].to_string();
    assert_eq!(history["undo"]["label"], "Delete prompt node");
    fixture.stop()?;
    fixture.json_output(
        fixture.list_command().env("PATH", "").output()?,
        "restart without jj",
    )?;
    fixture.success(&[
        "history",
        "undo",
        "--project",
        project,
        "--expected",
        &delete_id,
    ])?;
    assert_eq!(
        fixture.success(&["workflow", "graph", "--workflow", "1"])?,
        moved
    );
    fixture.success(&[
        "history",
        "undo",
        "--project",
        project,
        "--expected",
        &move_id,
    ])?;
    assert_eq!(
        fixture.success(&["workflow", "graph", "--workflow", "1"])?,
        before
    );
    fixture.success(&[
        "history",
        "redo",
        "--project",
        project,
        "--expected",
        &move_id,
    ])?;
    fixture.success(&[
        "history",
        "redo",
        "--project",
        project,
        "--expected",
        &delete_id,
    ])?;
    assert_eq!(
        fixture.success(&["workflow", "graph", "--workflow", "1"])?,
        deleted
    );
    for (method, params) in [
        (
            "node.move_prompt",
            json!({"workflow_id": 1, "node_id": 3, "parent": {"kind":"workflow_base"}, "acknowledgement":"accept_undo_boundary"}),
        ),
        (
            "node.delete_prompt",
            json!({"workflow_id": 1, "node_id": 3, "force": true}),
        ),
    ] {
        let mut bytes =
            serde_json::to_vec(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))?;
        bytes.push(b'\n');
        assert_eq!(fixture.rpc(&bytes)?["error"]["code"], -32602);
    }
    assert!(
        !fixture
            .command()
            .args(["node", "move-prompt", "--workflow", "1", "--node", "3"])
            .output()?
            .status
            .success()
    );
    Ok(())
}
