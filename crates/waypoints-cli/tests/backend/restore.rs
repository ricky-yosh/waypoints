use super::*;

#[test]
fn failure_before_snapshot_preserves_files_and_recovery_refuses_a_stale_checkout()
-> anyhow::Result<()> {
    let (fixture, agent, workspace) = run_fixture("remember saved")?;
    fs::write(workspace.join("login.txt"), "basic\n")?;
    let first = start_run(&fixture, &agent)?;
    let id = first["run"]["id"].to_string();
    let completed = wait_run(&fixture, &id, "completed")?;
    fs::write(workspace.join("login.txt"), "unsaved\n")?;
    let current = recorded_commit(&workspace)?;
    intercept_snapshots(&fixture, "exit 42")?;
    assert!(
        !fixture
            .command()
            .args(["run", "restore", "--run", &id])
            .output()?
            .status
            .success()
    );
    let attempts = fixture.success(&["run", "restore-status", "--run", &id])?;
    assert_eq!(attempts[0]["state"], "unresolved");
    assert!(attempts[0]["preserved"].is_null());
    assert_eq!(recorded_commit(&workspace)?, current);
    assert_eq!(
        fs::read_to_string(workspace.join("login.txt"))?,
        "unsaved\n"
    );
    // An external metadata rewrite leaves this checkout stale. Recovery must
    // not repair it or quietly acknowledge the stale filesystem.
    // Explicitly save the dirty files first: native update-stale on a dirty
    // historical checkout can itself create divergent revisions.
    jj(
        &workspace,
        &["log", "--no-graph", "-r", "@", "-T", "commit_id"],
    )?;
    let project = fixture.directory.path().join("project");
    let change = first["run"]["change_id"].as_str().expect("change");
    jj(
        &project,
        &[
            "--ignore-working-copy",
            "describe",
            "-r",
            change,
            "-m",
            "external",
        ],
    )?;
    fixture.stop()?;
    let operation = attempts[0]["id"].to_string();
    // Recovery is a live control, so explicitly restart by inspecting first.
    assert_eq!(
        fixture.success(&["run", "restore-status", "--run", &id])?,
        attempts
    );
    assert!(
        !fixture
            .command()
            .args([
                "run",
                "recover-restore",
                "--operation",
                &operation,
                "--confirm-processes-stopped",
            ])
            .output()?
            .status
            .success()
    );
    assert_eq!(
        fs::read_to_string(workspace.join("login.txt"))?,
        "unsaved\n"
    );
    assert_eq!(
        fixture.success(&["run", "restore-status", "--run", &id])?,
        attempts
    );
    jj(&workspace, &["workspace", "update-stale"])?;
    let recovered = fixture.success(&[
        "run",
        "recover-restore",
        "--operation",
        &operation,
        "--confirm-processes-stopped",
    ])?;
    assert_eq!(recovered["state"], "recovered");
    assert!(recovered["preserved"].is_null());
    assert_eq!(
        fixture.success(&["run", "status", "--run", &id])?["run"],
        completed["run"]
    );
    Ok(())
}

#[test]
fn restoring_a_saved_result_keeps_node_and_change_then_starts_a_fresh_agent_session()
-> anyhow::Result<()> {
    let (fixture, agent, workspace) = run_fixture("remember first")?;
    fs::write(workspace.join("login.txt"), "basic\n")?;
    let first = start_run(&fixture, &agent)?;
    let id = first["run"]["id"].to_string();
    let completed = wait_run(&fixture, &id, "completed")?;
    assert_eq!(completed["run"]["snapshot"]["status"], "saved");
    fs::write(workspace.join("login.txt"), "animations\n")?;
    let second = continue_run(&fixture, &id, "remember second")?;
    let second_id = second["run"]["id"].to_string();
    wait_run(&fixture, &second_id, "completed")?;
    let first_review = fixture.success(&["run", "review", "--run", &id])?;
    let second_review = fixture.success(&["run", "review", "--run", &second_id])?;
    let graph = fixture.success(&["workflow", "graph", "--workflow", "1"])?;
    fs::write(workspace.join("unsaved.txt"), "checkpoint me\n")?;
    for (method, params) in [
        ("run.restore", json!({"run_id": 1, "force": true})),
        ("run.restore", json!({"run_id": "1"})),
        ("run.restore_status", json!({"run_id": 1, "repair": true})),
        (
            "run.recover_restore",
            json!({"operation_id": 1, "confirmation": true}),
        ),
    ] {
        let response = fixture.rpc(
            format!(
                "{}\n",
                json!({
                    "jsonrpc":"2.0", "id":1, "method": method, "params":params,
                })
            )
            .as_bytes(),
        )?;
        assert_eq!(response["error"]["code"], -32602, "{response}");
    }
    // Auto-start after restart must not accidentally replay this mutation.
    fixture.stop()?;
    let restored = fixture.success(&["run", "restore", "--run", &id])?;
    assert_eq!(restored["state"], "completed");
    assert_eq!(restored["after"]["change_id"], first["run"]["change_id"]);
    assert_eq!(restored["source"], first_review["revision"]);
    assert_eq!(fs::read_to_string(workspace.join("login.txt"))?, "basic\n");
    assert!(!workspace.join("unsaved.txt").exists());
    let preserved = restored["preserved"]["commit_id"]
        .as_str()
        .expect("checkpoint");
    assert_eq!(
        jj(
            &workspace,
            &[
                "--ignore-working-copy",
                "file",
                "show",
                "-r",
                preserved,
                "unsaved.txt",
            ]
        )?,
        "checkpoint me\n"
    );
    for source in [&id, &second_id] {
        let refused = fixture
            .command()
            .args([
                "run",
                "continue",
                "--run",
                source,
                "--prompt",
                "wrong memory",
            ])
            .output()?;
        assert!(!refused.status.success());
        assert!(String::from_utf8(refused.stderr)?.contains("fresh run"));
    }
    assert_eq!(
        fixture.success(&["workflow", "graph", "--workflow", "1"])?,
        graph
    );
    assert_eq!(
        fixture.success(&["run", "review", "--run", &id])?,
        first_review
    );
    assert_eq!(
        fixture.success(&["run", "review", "--run", &second_id])?,
        second_review
    );
    fixture.stop()?;
    assert_eq!(
        fixture.success(&["run", "restore-status", "--run", &id])?,
        json!([restored])
    );
    let fresh = start_run(&fixture, &agent)?;
    let fresh_id = fresh["run"]["id"].to_string();
    let fresh = wait_run(&fixture, &fresh_id, "completed")?;
    assert_ne!(fresh["run"]["session_id"], first["run"]["session_id"]);
    assert!(fresh["run"]["continued_from"].is_null());
    assert_eq!(fresh["run"]["snapshot"]["status"], "saved");
    assert_eq!(fs::read_to_string(workspace.join("login.txt"))?, "basic\n");
    let next = continue_run(&fixture, &fresh_id, "remember fresh follow-up")?;
    wait_run(&fixture, &next["run"]["id"].to_string(), "completed")?;
    Ok(())
}

#[test]
fn uncertain_restore_survives_restart_blocks_mutations_and_requires_explicit_reconciliation()
-> anyhow::Result<()> {
    let (fixture, agent, workspace) = run_fixture("remember restore")?;
    fs::write(workspace.join("login.txt"), "basic\n")?;
    let first = start_run(&fixture, &agent)?;
    let id = first["run"]["id"].to_string();
    wait_run(&fixture, &id, "completed")?;
    fs::write(workspace.join("login.txt"), "later edits\n")?;
    // Real jj successfully restores, but its wrapper reports failure before the
    // backend can verify completion. Retrying would obscure the original outcome.
    intercept_snapshots(
        &fixture,
        r#"
case " $* " in
  *" restore "*) "$native" "$@"; exit 42 ;;
esac
"#,
    )?;
    let result = fixture
        .command()
        .args(["run", "restore", "--run", &id])
        .output()?;
    assert!(!result.status.success());
    let attempts = fixture.success(&["run", "restore-status", "--run", &id])?;
    assert_eq!(attempts.as_array().expect("attempts").len(), 1);
    let saved = &attempts[0];
    assert_eq!(saved["state"], "unresolved");
    assert!(saved["preserved"]["commit_id"].is_string());
    assert!(saved["after"].is_null());
    assert_eq!(fs::read_to_string(workspace.join("login.txt"))?, "basic\n");
    let operation = saved["id"].to_string();
    let current = recorded_commit(&workspace)?;
    fixture.stop()?;
    assert_eq!(
        fixture.success(&["run", "restore-status", "--run", &id])?,
        attempts
    );
    for args in [
        vec!["run", "restore", "--run", &id],
        vec![
            "node",
            "update",
            "--workflow",
            "1",
            "--node",
            "1",
            "--prompt",
            "blocked",
        ],
        vec![
            "run",
            "start",
            "--workflow",
            "1",
            "--node",
            "1",
            "--executable",
            agent.to_str().expect("agent"),
        ],
    ] {
        let result = fixture.command().args(args).output()?;
        assert!(!result.status.success());
        assert!(String::from_utf8(result.stderr)?.contains("unresolved"));
    }
    assert_eq!(recorded_commit(&workspace)?, current);
    let response = fixture.rpc(
        format!(
            "{}\n",
            json!({
                "jsonrpc":"2.0", "id":1, "method":"run.recover_restore",
                "params":{"operation_id":saved["id"]},
            })
        )
        .as_bytes(),
    )?;
    assert_eq!(response["error"]["code"], -32000);
    assert_eq!(
        fixture.success(&["run", "restore-status", "--run", &id])?,
        attempts
    );
    let recovered = fixture.success(&[
        "run",
        "recover-restore",
        "--operation",
        &operation,
        "--confirm-processes-stopped",
    ])?;
    assert_eq!(recovered["state"], "recovered");
    assert_eq!(recovered["after"]["commit_id"], current);
    assert_eq!(recovered["preserved"], saved["preserved"]);
    assert_eq!(recorded_commit(&workspace)?, current);
    assert_eq!(
        fixture.success(&[
            "run",
            "recover-restore",
            "--operation",
            &operation,
            "--confirm-processes-stopped",
        ])?,
        recovered
    );
    assert!(
        !fixture
            .command()
            .args(["run", "continue", "--run", &id, "--prompt", "old memory",])
            .output()?
            .status
            .success()
    );
    let fresh = start_run(&fixture, &agent)?;
    wait_run(&fixture, &fresh["run"]["id"].to_string(), "completed")?;
    Ok(())
}
