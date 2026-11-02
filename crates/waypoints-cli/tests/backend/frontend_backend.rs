use super::*;
use waypoints_backend::{Client, Config};
use waypoints_core::storage::StorageFailure;

#[test]
fn persistent_run_write_failures_do_not_skip_owned_process_cleanup() -> anyhow::Result<()> {
    for (condition, preparation_fails) in [
        ("NEW.active = 0", false),
        (
            "NEW.active = 0 OR json_extract(NEW.record, '$.state.status') = 'finalizing'",
            true,
        ),
    ] {
        let (fixture, agent, workspace) = run_fixture("hang")?;
        fixture.success(&["node", "create", "--workflow", "1", "--prompt", "complete"])?;
        fixture.success(&[
            "node",
            "create-change",
            "--workflow",
            "1",
            "--node",
            "2",
            "--base",
            "root()",
        ])?;
        let second = fixture.directory.path().join("second-workspace");
        fixture.success(&[
            "node",
            "workspace",
            "create",
            "--workflow",
            "1",
            "--node",
            "2",
            "--directory",
            second.to_str().expect("workspace path"),
        ])?;
        fixture.stop()?;
        let project = fixture.directory.path().join("project");
        let native_head = || {
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
                    "id",
                ],
            )
        };
        let before = native_head()?;
        tokio::runtime::Builder::new_current_thread()
            .build()?
            .block_on(async {
                let database =
                    turso::Builder::new_local(fixture.database.to_str().expect("database path"))
                        .build()
                        .await?;
                database
                    .connect()?
                    .execute(
                        &format!(
                            "CREATE TRIGGER reject_run_write BEFORE UPDATE ON agent_runs
                             WHEN {condition}
                             BEGIN SELECT RAISE(FAIL, 'injected run write failure'); END"
                        ),
                        (),
                    )
                    .await?;
                Ok::<_, anyhow::Error>(())
            })?;
        let log = fixture.directory.path().join("foreground.log");
        let mut server = OwnedServer(
            fixture
                .command()
                .arg("serve")
                .stdout(Stdio::null())
                .stderr(fs::File::create(&log)?)
                .spawn()?,
        );
        wait_for_file(&fixture.socket);
        start_run(&fixture, &agent)?;
        let _cleanup = OwnedTestAgentGroup(&workspace);
        wait_for_file(&workspace.join("descendant.pid"));
        fixture.success(&[
            "run",
            "start",
            "--workflow",
            "1",
            "--node",
            "2",
            "--executable",
            agent.to_str().expect("agent path"),
        ])?;
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = server.0.try_wait()? {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "failed persistence blocked shutdown: {}",
                fs::read_to_string(&log)?
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(!status.success());
        assert!(fs::read_to_string(&log)?.contains("injected run write failure"));
        assert_process_stopped(&workspace.join("adapter.pid"))?;
        assert_process_stopped(&workspace.join("descendant.pid"))?;
        assert_process_stopped(&second.join("adapter.pid"))?;
        let after = native_head()?;
        assert_eq!(
            after == before,
            preparation_fails,
            "snapshot must start only after preparation metadata was saved"
        );

        // Failed terminal writes must not pretend the reservation was released,
        // nor replay the successful agent or its unfinished snapshot on restart.
        for id in ["1", "2"] {
            let status = fixture.success(&["run", "status", "--run", id])?;
            assert_eq!(status["run"]["state"]["status"], "interrupted", "{status}");
            assert_eq!(status["run"]["snapshot"]["status"], "not_recorded");
            if id == "2" {
                assert_eq!(status["run"]["state"]["phase"], "finalization");
            }
            let output = fixture
                .command()
                .args([
                    "run",
                    "start",
                    "--workflow",
                    "1",
                    "--node",
                    id,
                    "--executable",
                ])
                .arg(&agent)
                .output()?;
            assert!(
                !output.status.success(),
                "lost node {id} reservation after restart"
            );
        }
        assert_eq!(native_head()?, after, "restart replayed native work");
    }
    Ok(())
}

// Clean up this test's freshly launched group even when the regression fails.
struct OwnedTestAgentGroup<'a>(&'a Path);

impl Drop for OwnedTestAgentGroup<'_> {
    fn drop(&mut self) {
        let child = self.0.join("descendant.pid");
        let Ok(pid) = fs::read_to_string(child) else {
            return;
        };
        let Ok(output) = Command::new("ps")
            .args(["-p", pid.trim(), "-o", "stat="])
            .output()
        else {
            return;
        };
        let state = String::from_utf8_lossy(&output.stdout);
        if output.status.success() && !state.trim().starts_with('Z') {
            let _ = stop_fixture_agent_group(self.0);
        }
    }
}

#[test]
fn named_rpc_parameters_and_error_diagnostics_are_stable_while_idle() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    fixture.start()?;
    for (method, params, code, message) in [
        (
            "workflow.get",
            json!({"workflow_id":123}),
            -32000,
            Some("workflow 123 not found"),
        ),
        (
            "repository.inspect",
            json!({"project_path":"."}),
            -32000,
            Some("project path must be absolute"),
        ),
        ("workflow.archive", json!([1]), -32602, None),
        ("node.workspace.update_stale", json!([1, 1]), -32602, None),
        ("run.cancel", json!([1]), -32602, None),
        ("run.status", json!([1]), -32602, None),
        ("does.not.exist", json!({}), -32601, None),
    ] {
        let response = fixture.rpc(
            format!(
                "{}\n",
                json!({
                    "jsonrpc":"2.0","id":7,"method":method,"params":params
                })
            )
            .as_bytes(),
        )?;
        assert_eq!(response["error"]["code"], code, "{response}");
        if let Some(message) = message {
            assert_eq!(response["error"]["message"], message);
        }
    }
    let output = fixture
        .command()
        .args(["workflow", "get", "--workflow", "123"])
        .output()?;
    assert!(!output.status.success());
    let diagnostic = String::from_utf8(output.stderr)?;
    assert_eq!(
        diagnostic.matches("workflow 123 not found").count(),
        1,
        "{diagnostic}"
    );
    Ok(())
}

#[test]
fn workspace_update_cli_is_explicit_verified_and_uncertainty_survives_restart() -> anyhow::Result<()>
{
    let (fixture, _, workspace) = run_fixture("No run")?;
    let project = fixture.directory.path().join("project");
    let saved = fixture.success(&["node", "refresh", "--workflow", "1", "--node", "1"])?;
    let change = saved["association"]["change_id"]
        .as_str()
        .expect("change ID");
    let update = [
        "node",
        "workspace",
        "update-stale",
        "--workflow",
        "1",
        "--node",
        "1",
    ];
    let native_head = || {
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
                "id",
            ],
        )
    };
    let head = native_head()?;
    assert_eq!(fixture.success(&update)?["outcome"], "already_fresh");
    assert_eq!(native_head()?, head);
    jj(
        &project,
        &[
            "--ignore-working-copy",
            "--at-op=@",
            "describe",
            "-r",
            change,
            "-m",
            "Rewrite",
        ],
    )?;
    assert_eq!(fixture.success(&update)?["outcome"], "updated");
    assert_eq!(fixture.success(&update)?["outcome"], "already_fresh");
    fixture.stop()?;
    assert_eq!(fixture.success(&update)?["outcome"], "already_fresh");
    jj(
        &project,
        &[
            "--ignore-working-copy",
            "--at-op=@",
            "describe",
            "-r",
            change,
            "-m",
            "Rewrite again",
        ],
    )?;
    intercept_snapshots(&fixture, "echo 'injected update failure' >&2\nexit 1")?;
    let head = native_head()?;
    let output = fixture.command().args(update).output()?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("do not replay"));
    fixture.stop()?;
    let status = fixture.success(&[
        "node",
        "workspace",
        "status",
        "--workflow",
        "1",
        "--node",
        "1",
    ])?;
    assert_eq!(status["state"]["status"], "incomplete");
    let output = fixture.command().args(update).output()?;
    assert!(!output.status.success());
    assert_eq!(native_head()?, head);
    assert!(
        !fixture
            .command()
            .args([
                "node",
                "workspace",
                "verify",
                "--workflow",
                "1",
                "--node",
                "1"
            ])
            .output()?
            .status
            .success()
    );
    jj(&workspace, &["workspace", "update-stale"])?;
    fixture.success(&[
        "node",
        "workspace",
        "verify",
        "--workflow",
        "1",
        "--node",
        "1",
    ])?;
    Ok(())
}

fn runtime() -> anyhow::Result<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?)
}

#[test]
fn structured_errors_preserve_domain_details_and_transport_distinction() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    fixture.start()?;
    runtime()?.block_on(async {
        let config = Config::new(&fixture.database, &fixture.socket)?;
        let client = Client::connect(&config).await?;
        let error = client.workflow(123).await.expect_err("missing workflow");
        let rpc = error
            .rpc_error()
            .expect("remote error must retain envelope");
        assert_eq!(rpc.code(), -32000);
        assert_eq!(
            rpc.failure(),
            Some(StorageFailure::MissingWorkflow { workflow_id: 123 })
        );
        let workflow = client
            .create_workflow(fixture.directory.path(), "Errors")
            .await?;
        let error = client
            .node(workflow.id, 456)
            .await
            .expect_err("missing node");
        assert_eq!(
            error.rpc_error().and_then(|error| error.failure()),
            Some(StorageFailure::MissingNode {
                workflow_id: workflow.id,
                node_id: 456
            })
        );
        let node = client.create_node(workflow.id, "No workspace").await?;
        let error = client
            .start_run(workflow.id, node.id, Path::new("/bin/sh"))
            .await
            .expect_err("no allocation");
        assert_eq!(
            error.rpc_error().and_then(|error| error.failure()),
            Some(StorageFailure::WorkspaceNotReady { node_id: node.id })
        );
        let error = client.run_status(789).await.expect_err("missing run");
        assert_eq!(
            error.rpc_error().and_then(|error| error.failure()),
            Some(StorageFailure::MissingRun { run_id: 789 })
        );
        // Unknown domain errors still retain useful diagnostics, not fabricated tags.
        let error = client
            .create_workflow(fixture.directory.path(), " ")
            .await
            .expect_err("blank name");
        assert!(error.rpc_error().is_some());
        assert!(
            error
                .rpc_error()
                .and_then(|error| error.failure())
                .is_none()
        );
        client.stop().await?;
        // Wait for the same owned backend to release the socket.
        let until = Instant::now() + Duration::from_secs(3);
        while fixture.socket.exists() && Instant::now() < until {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            client
                .workflow(workflow.id)
                .await
                .expect_err("closed socket")
                .rpc_error()
                .is_none()
        );
        Ok::<_, anyhow::Error>(())
    })?;
    Ok(())
}

fn block_inspection(fixture: &Fixture) -> anyhow::Result<(PathBuf, PathBuf)> {
    fixture.stop()?;
    let native = Command::new("/bin/sh")
        .args(["-c", "command -v jj"])
        .output()?;
    assert!(native.status.success());
    let native = String::from_utf8(native.stdout)?.trim().to_owned();
    let bin = fixture.directory.path().join("control-bin");
    fs::create_dir(&bin)?;
    let entered = fixture.directory.path().join("native-entered");
    let release = fixture.directory.path().join("native-release");
    let wrapper = bin.join("jj");
    fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nfor arg do\n if [ \"$arg\" = bookmark ]; then\n  /usr/bin/touch '{}'\n  while [ ! -e '{}' ]; do /bin/sleep 0.01; done\n fi\ndone\nexec '{}' \"$@\"\n",
            entered.display(),
            release.display(),
            native
        ),
    )?;
    fs::set_permissions(wrapper, fs::Permissions::from_mode(0o700))?;
    fixture.json_output(
        fixture
            .list_command()
            .env(
                "PATH",
                format!("{}:{}", bin.display(), std::env::var("PATH")?),
            )
            .output()?,
        "start controlled backend",
    )?;
    Ok((entered, release))
}

fn begin_inspection(fixture: &Fixture, entered: &Path) -> anyhow::Result<BufReader<UnixStream>> {
    let mut stream = UnixStream::connect(&fixture.socket)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.write_all(
        format!(
            "{}\n",
            json!({
                "jsonrpc":"2.0", "id":42, "method":"repository.inspect",
                "params":{"project_path":fixture.directory.path().join("project")}
            })
        )
        .as_bytes(),
    )?;
    wait_for_file(entered);
    Ok(BufReader::new(stream))
}

#[test]
fn live_controls_bypass_blocked_native_work_and_conflicting_mutations_do_not_start()
-> anyhow::Result<()> {
    for permit in [false, true] {
        let (fixture, agent, workspace) = run_fixture("permission")?;
        let (entered, release) = block_inspection(&fixture)?;
        let started = start_run(&fixture, &agent)?;
        let id = started["run"]["id"].to_string();
        let waiting = wait_run(&fixture, &id, "awaiting_permission")?;
        runtime()?.block_on(async {
            let client = Client::connect(&Config::new(&fixture.database, &fixture.socket)?).await?;
            let state = client.refresh_node(1, 1).await?;
            let change = state.association.expect("saved association").change_id;
            let error = client
                .associate_node(1, 1, &change)
                .await
                .expect_err("active node must stay reserved");
            assert_eq!(
                error.rpc_error().and_then(|error| error.failure()),
                Some(StorageFailure::NodeBusy {
                    node_id: 1,
                    run_id: 1
                })
            );
            Ok::<_, anyhow::Error>(())
        })?;
        let mut native = begin_inspection(&fixture, &entered)?;
        let begin = Instant::now();
        let status = fixture.success(&["run", "status", "--run", &id])?;
        assert_eq!(status["permission"], waiting["permission"]);
        let events = fixture.rpc(
            format!(
                "{}\n",
                json!({
                    "jsonrpc":"2.0","id":1,"method":"run.events","params":{"run_id":1,"after":0}
                })
            )
            .as_bytes(),
        )?;
        assert!(events["result"]["events"].is_array());
        let busy = fixture.rpc(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"workflow.rename\",\"params\":{\"workflow_id\":1,\"name\":\"Must not start\"}}\n")?;
        assert_eq!(busy["error"]["code"], -32001);
        assert_eq!(busy["error"]["data"]["kind"], "backend_busy");
        for (method, params, code) in [
            ("does.not.exist", json!({}), -32601),
            ("workflow.get", json!({"workflow_id":"bad"}), -32602),
            ("node.workspace.update_stale", json!([1, 1]), -32602),
            (
                "node.create",
                json!({"workflow_id":1,"prompt":"Bad","parent":{"kind":"workflow_base"},"parents":[]}),
                -32602,
            ),
        ] {
            let response = fixture.rpc(
                format!(
                    "{}\n",
                    json!({
                        "jsonrpc":"2.0","id":"invalid","method":method,"params":params
                    })
                )
                .as_bytes(),
            )?;
            assert_eq!(response["id"], "invalid");
            assert_eq!(response["error"]["code"], code, "{response}");
            assert!(response["error"].get("data").is_none());
        }
        if permit {
            let response = fixture.rpc(format!("{}\n", json!({
                "jsonrpc":"2.0","id":1,"method":"run.permission",
                "params":{"run_id":1,"permission_id":waiting["permission"]["id"],"option_id":"allow"}
            })).as_bytes())?;
            assert!(response.get("error").is_none(), "{response}");
            wait_for_file(&workspace.join("approved.txt"));
            // Completion is live, but snapshot preparation waits for storage.
            wait_run(&fixture, &id, "finalizing")?;
        } else {
            fixture.success(&["run", "cancel", "--run", &id])?;
            wait_run(&fixture, &id, "cancelled")?;
            assert!(!workspace.join("approved.txt").exists());
        }
        assert!(
            begin.elapsed() < Duration::from_millis(1500),
            "controls waited for native operation"
        );
        assert!(!release.exists());
        fs::write(release, "")?;
        let mut line = String::new();
        native.read_line(&mut line)?;
        let response: Value = serde_json::from_str(&line)?;
        assert!(response.get("error").is_none(), "{response}");
        if permit {
            wait_run(&fixture, &id, "completed")?;
        }
        assert_eq!(
            fixture.success(&["workflow", "get", "--workflow", "1"])?["name"],
            "Runs"
        );
        fixture.stop()?;
        let loaded = fixture.success(&["run", "status", "--run", &id])?;
        assert_eq!(
            loaded["run"]["state"]["status"],
            if permit { "completed" } else { "cancelled" }
        );
    }
    Ok(())
}

#[test]
fn shutdown_settles_blocked_native_request_before_releasing_backend_ownership() -> anyhow::Result<()>
{
    let (fixture, agent, _) = run_fixture("permission")?;
    let (entered, release) = block_inspection(&fixture)?;
    let started = start_run(&fixture, &agent)?;
    let id = started["run"]["id"].to_string();
    wait_run(&fixture, &id, "awaiting_permission")?;
    let mut native = begin_inspection(&fixture, &entered)?;
    let response = fixture
        .rpc(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"backend.shutdown\",\"params\":{}}\n")?;
    assert_eq!(response["result"], Value::Null);
    assert!(
        fixture.socket.exists(),
        "native job still owns database/socket"
    );
    fs::write(release, "")?;
    let mut line = String::new();
    native.read_line(&mut line)?;
    assert!(serde_json::from_str::<Value>(&line)?.get("error").is_none());
    let until = Instant::now() + Duration::from_secs(5);
    while fixture.socket.exists() {
        assert!(Instant::now() < until, "shutdown did not settle");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        fixture.success(&["run", "status", "--run", &id])?["run"]["state"]["status"],
        "cancelled"
    );
    Ok(())
}
