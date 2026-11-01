use super::*;

fn chain() -> anyhow::Result<(Fixture, PathBuf, PathBuf, PathBuf, Value, Value)> {
    let fixture = Fixture::new()?;
    jj(fixture.directory.path(), &["git", "init", "project"])?;
    let project = fixture.directory.path().join("project");
    jj(
        &project,
        &["config", "set", "--repo", "user.name", "Restore test"],
    )?;
    jj(
        &project,
        &[
            "config",
            "set",
            "--repo",
            "user.email",
            "test@example.invalid",
        ],
    )?;
    fixture.success(&[
        "workflow",
        "create",
        "--project",
        project.to_str().unwrap(),
        "--name",
        "Downstream",
        "--base-revision",
        "root()",
    ])?;
    let agent = fixture.directory.path().join("fake-acp");
    fs::write(&agent, include_str!("../fixtures/acp_agent.py"))?;
    fs::set_permissions(&agent, fs::Permissions::from_mode(0o700))?;
    let source_dir = fixture.directory.path().join("source");
    let child_dir = fixture.directory.path().join("child");
    let mut runs = Vec::new();
    for (id, prompt, directory) in [
        ("1", "remember API", &source_dir),
        ("2", "remember login", &child_dir),
    ] {
        let mut args = vec!["node", "create", "--workflow", "1", "--prompt", prompt];
        if id == "1" {
            args.push("--from-base");
        } else {
            args.extend(["--parent", "1"]);
        }
        fixture.success(&args)?;
        fixture.success(&["node", "create-change", "--workflow", "1", "--node", id])?;
        fixture.success(&[
            "node",
            "workspace",
            "create",
            "--workflow",
            "1",
            "--node",
            id,
            "--directory",
            directory.to_str().unwrap(),
        ])?;
        if id == "1" {
            fs::write(directory.join("api.txt"), "API A\n")?;
        }
        let started = fixture.success(&[
            "run",
            "start",
            "--workflow",
            "1",
            "--node",
            id,
            "--executable",
            agent.to_str().unwrap(),
        ])?;
        runs.push(wait_run(
            &fixture,
            &started["run"]["id"].to_string(),
            "completed",
        )?);
        if id == "1" {
            fs::write(directory.join("api.txt"), "API B\n")?;
            jj(directory, &["describe", "-m", "API B"])?;
        }
    }
    fixture.success(&[
        "node",
        "create",
        "--workflow",
        "1",
        "--parent",
        "2",
        "--prompt",
        "Test login",
    ])?;
    fs::write(source_dir.join("api.txt"), "unsaved API C\n")?;
    fs::write(child_dir.join("login.txt"), "unsaved login\n")?;
    let child_run = runs.pop().unwrap();
    let source_run = runs.pop().unwrap();
    Ok((fixture, agent, source_dir, child_dir, source_run, child_run))
}

fn preview(fixture: &Fixture, source_run: &Value) -> anyhow::Result<(PathBuf, Value)> {
    let selected = source_run["run"]["snapshot"]["revision"]["commit_id"]
        .as_str()
        .unwrap();
    let preview = fixture.success(&[
        "node",
        "restore-impact",
        "--workflow",
        "1",
        "--node",
        "1",
        "--commit",
        selected,
    ])?;
    let path = fixture.directory.path().join("preview.json");
    fs::write(&path, serde_json::to_vec(&preview)?)?;
    Ok((path, preview))
}

fn apply(fixture: &Fixture, path: &Path) -> anyhow::Result<Output> {
    Ok(fixture
        .command()
        .args([
            "node",
            "restore-with-descendants",
            "--preview",
            path.to_str().unwrap(),
            "--confirm-restore-and-rebase",
        ])
        .output()?)
}

#[test]
fn confirmed_cli_restore_rebases_without_rerunning_and_fresh_child_sessions_are_explicit()
-> anyhow::Result<()> {
    let (fixture, agent, source_dir, child_dir, source_run, child_run) = chain()?;
    let (path, expected) = preview(&fixture, &source_run)?;
    let missing_flag = fixture
        .command()
        .args([
            "node",
            "restore-with-descendants",
            "--preview",
            path.to_str().unwrap(),
        ])
        .output()?;
    assert_eq!(missing_flag.status.code(), Some(2));
    for (params, code) in [
        (json!({"preview":expected}), -32000),
        (
            json!({"preview":expected,"confirmation":"processes_stopped"}),
            -32602,
        ),
        (
            json!({"preview":expected,"confirmation":"restore_and_rebase","force":true}),
            -32602,
        ),
        (
            json!({"preview":null,"confirmation":"restore_and_rebase"}),
            -32602,
        ),
    ] {
        let response = fixture.rpc(
            format!(
                "{}\n",
                json!({
                    "jsonrpc":"2.0","id":1,"method":"node.restore_with_descendants","params":params,
                })
            )
            .as_bytes(),
        )?;
        assert_eq!(response["error"]["code"], code, "{response}");
    }
    fixture.success(&[
        "node",
        "update",
        "--workflow",
        "1",
        "--node",
        "2",
        "--prompt",
        "remember changed login assignment",
    ])?;
    let stale = apply(&fixture, &path)?;
    assert!(!stale.status.success());
    assert!(String::from_utf8_lossy(&stale.stderr).contains("preview changed"));
    assert_eq!(
        fixture.success(&["node", "restores", "--workflow", "1", "--node", "1"])?,
        json!([])
    );
    let (path, _) = preview(&fixture, &source_run)?;
    let graph = fixture.success(&["workflow", "graph", "--workflow", "1"])?;
    let project = fixture.directory.path().join("project");
    let history =
        fixture.success(&["history", "status", "--project", project.to_str().unwrap()])?;
    let original = recorded_commit(&project)?;
    let restored = fixture.json_output(apply(&fixture, &path)?, "confirmed downstream restore")?;
    assert_eq!(restored["state"], "completed");
    assert!(restored["run_id"].is_null());
    assert_eq!(restored["downstream"]["nodes"].as_array().unwrap().len(), 3);
    let child = restored["downstream"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["node_id"] == 2)
        .unwrap();
    assert_eq!(child["needs_workspace_update"], true);
    assert_eq!(child["after"]["conflicted"], false);
    assert!(
        restored["downstream"]["context_notice"]
            .as_str()
            .unwrap()
            .contains("not rerun")
    );
    assert_eq!(fs::read_to_string(source_dir.join("api.txt"))?, "API A\n");
    assert_eq!(fs::read_to_string(child_dir.join("api.txt"))?, "API B\n");
    assert_eq!(
        fs::read_to_string(child_dir.join("login.txt"))?,
        "unsaved login\n"
    );
    assert_eq!(
        fixture.success(&["workflow", "graph", "--workflow", "1"])?,
        graph
    );
    assert_eq!(
        fixture.success(&["history", "status", "--project", project.to_str().unwrap()])?,
        history
    );
    assert_eq!(recorded_commit(&project)?, original);
    assert_eq!(
        agent_requests(&agent)?
            .iter()
            .filter(|r| r["method"] == "session/new")
            .count(),
        2
    );
    fixture.stop()?;
    assert_eq!(
        fixture.success(&["node", "restores", "--workflow", "1", "--node", "2"])?,
        json!([restored])
    );
    for stopped in [&source_run, &child_run] {
        let id = stopped["run"]["id"].to_string();
        let refused = fixture
            .command()
            .args([
                "run",
                "continue",
                "--run",
                &id,
                "--prompt",
                "Use old memory",
            ])
            .output()?;
        assert!(!refused.status.success());
        assert!(String::from_utf8_lossy(&refused.stderr).contains("fresh run"));
        assert_eq!(fixture.success(&["run", "status", "--run", &id])?, *stopped);
    }
    let refused = fixture
        .command()
        .args([
            "run",
            "start",
            "--workflow",
            "1",
            "--node",
            "2",
            "--executable",
            agent.to_str().unwrap(),
        ])
        .output()?;
    assert!(
        !refused.status.success(),
        "stale descendant checkout must not launch an agent"
    );
    jj(&child_dir, &["workspace", "update-stale"])?;
    let fresh = fixture.success(&[
        "run",
        "start",
        "--workflow",
        "1",
        "--node",
        "2",
        "--executable",
        agent.to_str().unwrap(),
    ])?;
    let fresh = wait_run(&fixture, &fresh["run"]["id"].to_string(), "completed")?;
    assert!(fresh["run"]["continued_from"].is_null());
    let requests = agent_requests(&agent)?;
    assert_eq!(
        requests
            .iter()
            .filter(|r| r["method"] == "session/new")
            .count(),
        3
    );
    assert!(!requests.iter().any(|r| r["method"] == "session/load"));
    Ok(())
}

#[test]
fn backend_crashes_leave_downstream_checkpoints_and_barriers_until_explicit_recovery()
-> anyhow::Result<()> {
    for window in ["before_snapshot", "after_child_checkpoint", "after_restore"] {
        let (fixture, _agent, source_dir, child_dir, source_run, child_run) = chain()?;
        let (path, _) = preview(&fixture, &source_run)?;
        let script = match window {
            "before_snapshot" => "kill -9 \"$PPID\"; exit 42".to_owned(),
            "after_child_checkpoint" => format!(
                "if [ \"$(pwd -P)\" = '{}' ]; then kill -9 \"$PPID\"; exit 42; fi",
                source_dir.canonicalize()?.display()
            ),
            "after_restore" => r#"case " $* " in
                *" restore "*) "$native" "$@"; kill -9 "$PPID"; exit 42 ;;
                esac"#
                .to_owned(),
            _ => unreachable!(),
        };
        intercept_snapshots(&fixture, &script)?;
        assert!(!apply(&fixture, &path)?.status.success(), "{window}");
        // This request starts a replacement backend; it never repeats the mutation.
        let attempts = fixture.success(&["node", "restores", "--workflow", "1", "--node", "2"])?;
        let attempt = &attempts[0];
        assert_eq!(attempt["state"], "unresolved", "{window}");
        assert_eq!(attempt["downstream"]["nodes"].as_array().unwrap().len(), 3);
        let child = attempt["downstream"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["node_id"] == 2)
            .unwrap();
        assert_eq!(
            child["preserved"].is_object(),
            window != "before_snapshot",
            "{window}"
        );
        assert_eq!(
            attempt["preserved"].is_object(),
            window == "after_restore",
            "{window}"
        );
        assert!(
            !apply(&fixture, &path)?.status.success(),
            "no automatic or explicit replay through guard"
        );
        let edited = fixture
            .command()
            .args([
                "node",
                "update",
                "--workflow",
                "1",
                "--node",
                "1",
                "--prompt",
                "blocked",
            ])
            .output()?;
        assert!(!edited.status.success(), "global mutation guard retained");
        if window == "after_restore" {
            let refused = fixture
                .command()
                .args([
                    "run",
                    "recover-restore",
                    "--operation",
                    "1",
                    "--confirm-processes-stopped",
                ])
                .output()?;
            assert!(
                !refused.status.success(),
                "stale descendant must block recovery"
            );
            jj(&child_dir, &["workspace", "update-stale"])?;
        }
        let project = fixture.directory.path().join("project");
        let before = jj(
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
                "id",
            ],
        )?;
        let recovered = fixture.success(&[
            "run",
            "recover-restore",
            "--operation",
            "1",
            "--confirm-processes-stopped",
        ])?;
        assert_eq!(recovered["state"], "recovered");
        assert_eq!(
            fixture.success(&[
                "run",
                "recover-restore",
                "--operation",
                "1",
                "--confirm-processes-stopped"
            ])?,
            recovered
        );
        assert_eq!(
            jj(
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
                    "id"
                ]
            )?,
            before,
            "recovery must not snapshot, restore or repair"
        );
        assert_eq!(
            fs::read_to_string(source_dir.join("api.txt"))?,
            if window == "after_restore" {
                "API A\n"
            } else {
                "unsaved API C\n"
            }
        );
        for stopped in [&source_run, &child_run] {
            let id = stopped["run"]["id"].to_string();
            let refused = fixture
                .command()
                .args(["run", "continue", "--run", &id, "--prompt", "old memory"])
                .output()?;
            assert!(!refused.status.success());
            assert!(String::from_utf8_lossy(&refused.stderr).contains("fresh run"));
            assert_eq!(fixture.success(&["run", "status", "--run", &id])?, *stopped);
        }
    }
    Ok(())
}
