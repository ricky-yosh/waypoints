use super::*;

#[test]
fn native_revision_cli_pages_include_saved_runs_and_manual_edits_with_exact_readonly_previews()
-> anyhow::Result<()> {
    let (fixture, agent, workspace) = run_fixture("remember history")?;
    let first = start_run(&fixture, &agent)?;
    let run_id = first["run"]["id"].to_string();
    let completed = wait_run(&fixture, &run_id, "completed")?;
    let saved = completed["run"]["snapshot"]["revision"]["commit_id"]
        .as_str()
        .expect("saved commit");
    let saved_review = fixture.success(&["run", "review", "--run", &run_id])?;
    for index in 0..5 {
        fs::write(
            workspace.join("manual.txt"),
            format!("manual version {index}\n"),
        )?;
        jj(&workspace, &["describe", "-m", &format!("manual {index}")])?;
    }
    let current = recorded_commit(&workspace)?;
    fs::write(workspace.join("dirty.txt"), "keep unrecorded\n")?;
    let graph = fixture.success(&["workflow", "graph", "--workflow", "1"])?;
    let first = fixture.success(&[
        "node",
        "revisions",
        "--workflow",
        "1",
        "--node",
        "1",
        "--page-size",
        "2",
    ])?;
    assert_eq!(first["entries"].as_array().unwrap().len(), 2);
    assert_eq!(first["current_commit_ids"], json!([current]));
    assert_eq!(first["entries"][0]["is_current"], true);
    assert_eq!(first["entries"][0]["run_ids"], json!([]));
    let mut cursor = first["next_cursor"]
        .as_str()
        .expect("next cursor")
        .to_owned();
    let mut entries = first["entries"].as_array().unwrap().clone();
    fixture.stop()?;
    loop {
        let page = fixture.success(&[
            "node",
            "revisions",
            "--workflow",
            "1",
            "--node",
            "1",
            "--page-size",
            "2",
            "--cursor",
            &cursor,
        ])?;
        assert_eq!(page["native_operation"], first["native_operation"]);
        entries.extend(page["entries"].as_array().unwrap().iter().cloned());
        match page["next_cursor"].as_str() {
            Some(next) => cursor = next.to_owned(),
            None => break,
        }
    }
    let entry = entries
        .iter()
        .find(|entry| entry["evolution"]["revision"]["commit_id"] == saved)
        .expect("saved run appears in native evolution");
    assert_eq!(entry["run_ids"], json!([completed["run"]["id"]]));
    assert_eq!(
        fixture.success(&[
            "node",
            "review",
            "--workflow",
            "1",
            "--node",
            "1",
            "--commit",
            saved,
        ])?,
        saved_review
    );
    assert_eq!(
        fixture.success(&["workflow", "graph", "--workflow", "1"])?,
        graph
    );
    assert_eq!(recorded_commit(&workspace)?, current);
    assert_eq!(
        fs::read_to_string(workspace.join("dirty.txt"))?,
        "keep unrecorded\n"
    );
    let human = Command::new(env!("CARGO_BIN_EXE_waypoints"))
        .arg("--database")
        .arg(&fixture.database)
        .arg("--socket")
        .arg(&fixture.socket)
        .args([
            "node",
            "revisions",
            "--workflow",
            "1",
            "--node",
            "1",
            "--page-size",
            "1",
        ])
        .output()?;
    assert!(human.status.success());
    let human = String::from_utf8(human.stdout)?;
    assert!(human.contains(&current) && human.contains("current") && human.contains("Next cursor"));
    for (method, params, code) in [
        (
            "node.revisions",
            json!({"workflow_id":1,"node_id":1,"page_size":0}),
            -32000,
        ),
        (
            "node.revisions",
            json!({"workflow_id":1,"node_id":1,"page_size":101}),
            -32000,
        ),
        (
            "node.revisions",
            json!({"workflow_id":1,"node_id":1,"page_size":-1}),
            -32602,
        ),
        (
            "node.revisions",
            json!({"workflow_id":1,"node_id":1,"page_size":"2"}),
            -32602,
        ),
        (
            "node.revisions",
            json!({"workflow_id":1,"node_id":1,"page_size":1,"force":true}),
            -32602,
        ),
        (
            "node.revisions",
            json!({"workflow_id":1,"node_id":1,"page_size":1,"cursor":"{}"}),
            -32000,
        ),
        (
            "node.revision_review",
            json!({"workflow_id":1,"node_id":1,"commit_id":"@"}),
            -32000,
        ),
        (
            "node.revision_review",
            json!({"workflow_id":1,"node_id":1,"commit_id":saved,"restore":true}),
            -32602,
        ),
    ] {
        let response = fixture.rpc(
            format!(
                "{}\n",
                json!({
                    "jsonrpc":"2.0","id":1,"method":method,"params":params,
                })
            )
            .as_bytes(),
        )?;
        assert_eq!(response["error"]["code"], code, "{response}");
    }
    assert_eq!(recorded_commit(&workspace)?, current);
    Ok(())
}
