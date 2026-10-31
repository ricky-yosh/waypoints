#![cfg(unix)]

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tempfile::TempDir;

struct Fixture {
    directory: TempDir,
    database: PathBuf,
    socket: PathBuf,
}

impl Fixture {
    fn new() -> anyhow::Result<Self> {
        // macOS limits Unix socket paths to about 100 bytes. The test runner's
        // TMPDIR may be much longer, so use a short, uniquely-created directory.
        let directory = tempfile::Builder::new().prefix("wp-").tempdir_in("/tmp")?;
        let database = directory.path().join("workflow.db");
        let socket = directory.path().join("backend.sock");
        Ok(Self {
            directory,
            database,
            socket,
        })
    }

    fn command_at(&self, socket: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_waypoints"));
        command
            .arg("--database")
            .arg(&self.database)
            .arg("--socket")
            .arg(socket)
            .arg("--json");
        command
    }

    fn command(&self) -> Command {
        self.command_at(&self.socket)
    }

    fn success(&self, args: &[&str]) -> anyhow::Result<Value> {
        let output = self.command().args(args).output()?;
        assert!(
            output.status.success(),
            "CLI failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(serde_json::from_slice(&output.stdout)?)
    }

    fn list_command(&self) -> Command {
        let mut command = self.command();
        command
            .args(["workflow", "list", "--project"])
            .arg(self.directory.path());
        command
    }

    fn start(&self) -> anyhow::Result<()> {
        let output = self.list_command().output()?;
        assert!(
            output.status.success(),
            "startup failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(())
    }

    fn stop(&self) -> anyhow::Result<()> {
        self.success(&["backend", "stop"])?;
        let deadline = Instant::now() + Duration::from_secs(3);
        while self.socket.exists() {
            assert!(
                Instant::now() < deadline,
                "backend failed to remove its socket"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }

    fn rpc(&self, bytes: &[u8]) -> anyhow::Result<Value> {
        let mut stream = UnixStream::connect(&self.socket)?;
        stream.set_read_timeout(Some(Duration::from_secs(7)))?;
        stream.write_all(bytes)?;
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line)?;
        Ok(serde_json::from_str(&line)?)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Clean up the real background process even when an assertion unwinds.
        let _ = self.command().args(["backend", "stop"]).output();
    }
}

struct OwnedServer(Child);

fn jj(project: &Path, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("jj")
        .current_dir(project)
        .env("JJ_CONFIG", "")
        .args([
            "--config",
            "user.name='Waypoints test'",
            "--config",
            "user.email='test@example.invalid'",
            "--no-pager",
            "--color=never",
        ])
        .args(args)
        .output()?;
    anyhow::ensure!(
        output.status.success(),
        "jj {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?)
}

#[test]
fn cli_creates_a_change_without_switching_and_retains_it_after_restart() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    jj(fixture.directory.path(), &["git", "init", "project"])?;
    let project = fixture.directory.path().join("project");
    jj(&project, &["describe", "-m", "Interfaces"])?;
    let base = jj(
        &project,
        &["log", "--no-graph", "-r", "@", "-T", "change_id"],
    )?;
    let before = jj(
        &project,
        &["log", "--no-graph", "-r", "@", "-T", "commit_id"],
    )?;
    fs::write(project.join("dirty.txt"), "untouched\n")?;
    let workflow = fixture.success(&[
        "workflow",
        "create",
        "--project",
        project.to_str().expect("project path"),
        "--name",
        "Creation",
    ])?["id"]
        .to_string();
    let node = fixture.success(&[
        "node",
        "create",
        "--workflow",
        &workflow,
        "--prompt",
        "Implement authentication",
    ])?;
    let node_id = node["id"].to_string();
    let created = fixture.success(&[
        "node",
        "create-change",
        "--workflow",
        &workflow,
        "--node",
        &node_id,
        "--base",
        &base,
    ])?;
    assert_eq!(created["node"], node);
    assert_eq!(created["state"]["status"], "current");
    let id = created["association"]["change_id"]
        .as_str()
        .expect("created change ID");
    assert_ne!(id, base);
    assert_eq!(
        jj(
            &project,
            &[
                "--ignore-working-copy",
                "log",
                "--no-graph",
                "-r",
                "@",
                "-T",
                "commit_id"
            ]
        )?,
        before
    );
    assert_eq!(
        fs::read_to_string(project.join("dirty.txt"))?,
        "untouched\n"
    );
    let operation = jj(
        &project,
        &[
            "--ignore-working-copy",
            "op",
            "log",
            "--no-graph",
            "--limit",
            "1",
            "-T",
            "id",
        ],
    )?;
    let rejected = fixture
        .command()
        .args([
            "node",
            "create-change",
            "--workflow",
            &workflow,
            "--node",
            &node_id,
            "--base",
            &base,
        ])
        .output()?;
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("already has"));
    assert_eq!(
        jj(
            &project,
            &[
                "--ignore-working-copy",
                "op",
                "log",
                "--no-graph",
                "--limit",
                "1",
                "-T",
                "id"
            ]
        )?,
        operation
    );
    for params in [
        json!({"workflow_id": 1, "node_id": 1}),
        json!({"workflow_id": 1, "node_id": 1, "base": "root()", "unexpected": true}),
    ] {
        let response = fixture.rpc(
            format!(
                "{}\n",
                json!({
                    "jsonrpc": "2.0", "id": 1, "method": "node.create_change", "params": params,
                })
            )
            .as_bytes(),
        )?;
        assert_eq!(response["error"]["code"], -32602);
    }
    fixture.stop()?;
    assert_eq!(
        fixture.success(&[
            "node",
            "refresh",
            "--workflow",
            &workflow,
            "--node",
            &node_id,
        ])?,
        created
    );
    fixture.stop()?;
    Ok(())
}

#[test]
fn uncertain_creation_is_guarded_across_restart_and_can_be_explicitly_associated()
-> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    jj(fixture.directory.path(), &["git", "init", "project"])?;
    let project = fixture.directory.path().join("project");
    jj(&project, &["describe", "-m", "Base"])?;
    let executable = Command::new("/bin/sh")
        .args(["-c", "command -v jj"])
        .output()?;
    assert!(executable.status.success());
    let native_jj = String::from_utf8(executable.stdout)?.trim().to_owned();
    let bin = fixture.directory.path().join("bin");
    fs::create_dir(&bin)?;
    let wrapper = bin.join("jj");
    fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nfor arg do\n if [ \"$arg\" = new ]; then\n  '{native_jj}' \"$@\"\n  exit 1\n fi\ndone\nexec '{native_jj}' \"$@\"\n"
        ),
    )?;
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700))?;
    // Only the backend receives the injected jj wrapper.
    assert!(
        fixture
            .list_command()
            .env("PATH", &bin)
            .output()?
            .status
            .success()
    );
    let workflow = fixture.success(&[
        "workflow",
        "create",
        "--project",
        project.to_str().expect("project path"),
        "--name",
        "Uncertain",
    ])?["id"]
        .to_string();
    let node = fixture.success(&[
        "node",
        "create",
        "--workflow",
        &workflow,
        "--prompt",
        "Keep prompt",
    ])?["id"]
        .to_string();
    let args = [
        "node",
        "create-change",
        "--workflow",
        &workflow,
        "--node",
        &node,
        "--base",
        "root()",
    ];
    let failure = fixture.command().args(args).output()?;
    assert!(!failure.status.success());
    assert!(String::from_utf8_lossy(&failure.stderr).contains("may have succeeded"));
    let operation = jj(
        &project,
        &[
            "--ignore-working-copy",
            "op",
            "log",
            "--no-graph",
            "--limit",
            "1",
            "-T",
            "id",
        ],
    )?;
    fixture.stop()?;
    let retry = fixture.command().args(args).output()?;
    assert!(!retry.status.success());
    assert!(String::from_utf8_lossy(&retry.stderr).contains("unfinished creation"));
    assert_eq!(
        jj(
            &project,
            &[
                "--ignore-working-copy",
                "op",
                "log",
                "--no-graph",
                "--limit",
                "1",
                "-T",
                "id"
            ]
        )?,
        operation
    );
    let created_id = jj(
        &project,
        &[
            "--ignore-working-copy",
            "log",
            "--no-graph",
            "-r",
            "heads(all()) ~ @",
            "-T",
            "change_id",
        ],
    )?;
    assert_eq!(created_id.len(), 32, "exactly one change was created");
    let selected = fixture.success(&[
        "node",
        "associate",
        "--workflow",
        &workflow,
        "--node",
        &node,
        "--change",
        &created_id,
    ])?;
    assert_eq!(selected["association"]["change_id"], created_id);
    assert_eq!(
        fixture.success(&["node", "refresh", "--workflow", &workflow, "--node", &node,])?,
        selected
    );
    fixture.stop()?;
    Ok(())
}

#[test]
fn slow_jj_times_out_without_blocking_subsequent_backend_requests() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let bin = fixture.directory.path().join("bin");
    fs::create_dir(&bin)?;
    let executable = bin.join("jj");
    // exec ensures kill-on-drop targets the sleeping process itself, not a wrapper.
    fs::write(&executable, "#!/bin/sh\nexec /bin/sleep 30\n")?;
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))?;
    let output = fixture.list_command().env("PATH", &bin).output()?;
    assert!(output.status.success());
    let project = fixture
        .directory
        .path()
        .to_str()
        .expect("temporary path is UTF-8");
    let workflow = fixture.success(&[
        "workflow",
        "create",
        "--project",
        project,
        "--name",
        "Slow jj",
    ])?["id"]
        .to_string();
    let node = fixture.success(&[
        "node",
        "create",
        "--workflow",
        &workflow,
        "--prompt",
        "Prompt",
    ])?["id"]
        .to_string();
    let output = fixture
        .command()
        .args([
            "node",
            "associate",
            "--workflow",
            &workflow,
            "--node",
            &node,
            "--change",
            "kkkk",
        ])
        .output()?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("jj command timed out"));
    assert_eq!(
        fixture.success(&["node", "refresh", "--workflow", &workflow, "--node", &node,])?["state"]
            ["status"],
        "unassociated"
    );
    fixture.success(&["backend", "status"])?;
    fixture.stop()?;
    Ok(())
}

#[test]
fn missing_jj_is_an_error_not_a_missing_change_and_backend_stays_usable() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let output = fixture.list_command().env("PATH", "").output()?;
    assert!(
        output.status.success(),
        "basic operations must not require jj"
    );
    let project = fixture
        .directory
        .path()
        .to_str()
        .expect("temporary path is UTF-8");
    let workflow = fixture.success(&[
        "workflow",
        "create",
        "--project",
        project,
        "--name",
        "No jj installed",
    ])?["id"]
        .to_string();
    let node = fixture.success(&[
        "node",
        "create",
        "--workflow",
        &workflow,
        "--prompt",
        "No jj yet",
    ])?["id"]
        .to_string();
    let output = fixture
        .command()
        .args([
            "node",
            "associate",
            "--workflow",
            &workflow,
            "--node",
            &node,
            "--change",
            "kkkk",
        ])
        .output()?;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("install jj"));
    assert_eq!(
        fixture.success(&["node", "refresh", "--workflow", &workflow, "--node", &node,])?["state"]
            ["status"],
        "unassociated"
    );
    fixture.success(&["backend", "status"])?;
    fixture.stop()?;
    Ok(())
}

#[test]
fn cli_associates_refreshes_and_preserves_missing_native_changes() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    jj(fixture.directory.path(), &["git", "init", "project"])?;
    let project = fixture.directory.path().join("project").canonicalize()?;
    jj(&project, &["describe", "-m", "Initial native change"])?;
    let id = jj(
        &project,
        &["log", "--no-graph", "-r", "@", "-T", "change_id"],
    )?;
    let workflow = fixture.success(&[
        "workflow",
        "create",
        "--project",
        project.to_str().expect("temporary path is UTF-8"),
        "--name",
        "jj-backed workflow",
    ])?["id"]
        .to_string();
    let node = fixture.success(&[
        "node",
        "create",
        "--workflow",
        &workflow,
        "--prompt",
        "Keep the prompt",
    ])?;
    let node_id = node["id"].to_string();
    assert_eq!(
        fixture.success(&[
            "node",
            "refresh",
            "--workflow",
            &workflow,
            "--node",
            &node_id
        ])?["state"]["status"],
        "unassociated"
    );
    fs::write(project.join("dirty.txt"), "not yet snapshotted")?;
    let op_args = [
        "--ignore-working-copy",
        "--at-op=@",
        "op",
        "log",
        "--no-graph",
        "--limit",
        "1",
        "-T",
        "id",
    ];
    let operation = jj(&project, &op_args)?;
    let associated = fixture.success(&[
        "node",
        "associate",
        "--workflow",
        &workflow,
        "--node",
        &node_id,
        "--change",
        &id[..12],
    ])?;
    assert_eq!(associated["node"], node);
    assert_eq!(associated["association"]["change_id"], id);
    assert_eq!(
        associated["association"]["repository_path"],
        project.to_str().expect("temporary path is UTF-8")
    );
    assert_eq!(associated["state"]["status"], "current");
    assert_eq!(associated["state"]["revision"]["conflicted"], false);
    assert_eq!(
        jj(&project, &op_args)?,
        operation,
        "backend queries must not snapshot"
    );
    fixture.stop()?;
    assert_eq!(
        fixture.success(&[
            "node",
            "refresh",
            "--workflow",
            &workflow,
            "--node",
            &node_id
        ])?,
        associated
    );
    jj(&project, &["describe", "-m", "Manually revised"])?;
    let refreshed = fixture.success(&[
        "node",
        "refresh",
        "--workflow",
        &workflow,
        "--node",
        &node_id,
    ])?;
    assert_eq!(
        refreshed["state"]["revision"]["description"],
        "Manually revised\n"
    );
    assert_ne!(
        refreshed["state"]["revision"]["commit_id"],
        associated["state"]["revision"]["commit_id"]
    );
    assert_eq!(refreshed["association"], associated["association"]);
    for args in [
        vec![
            "node",
            "associate",
            "--workflow",
            &workflow,
            "--node",
            &node_id,
            "--change",
            "all()",
        ],
        vec![
            "node",
            "associate",
            "--workflow",
            "99999",
            "--node",
            &node_id,
            "--change",
            &id,
        ],
        vec!["node", "refresh", "--workflow", "99999", "--node", &node_id],
    ] {
        let output = fixture.command().args(&args).output()?;
        assert!(!output.status.success(), "{args:?}");
        assert!(output.stdout.is_empty());
    }
    for (method, params) in [
        (
            "node.associate",
            json!({"workflow_id": 1, "node_id": 1, "change_id": 123}),
        ),
        (
            "node.refresh",
            json!({"workflow_id": 1, "node_id": 1, "extra": true}),
        ),
    ] {
        let mut bytes = serde_json::to_vec(
            &json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}),
        )?;
        bytes.push(b'\n');
        assert_eq!(fixture.rpc(&bytes)?["error"]["code"], -32602);
    }
    assert_eq!(
        fixture.success(&[
            "node",
            "refresh",
            "--workflow",
            &workflow,
            "--node",
            &node_id
        ])?,
        refreshed
    );
    jj(&project, &["abandon", "-r", &id])?;
    let operation = jj(&project, &op_args)?;
    let missing = fixture.success(&[
        "node",
        "refresh",
        "--workflow",
        &workflow,
        "--node",
        &node_id,
    ])?;
    assert_eq!(missing["state"]["status"], "missing");
    assert_eq!(missing["node"], node);
    assert_eq!(missing["association"], associated["association"]);
    assert_eq!(
        fixture.success(&["workflow", "graph", "--workflow", &workflow])?["nodes"],
        json!([node])
    );
    let output = fixture
        .command()
        .args([
            "node",
            "refresh",
            "--workflow",
            &workflow,
            "--node",
            &node_id,
        ])
        .output()?;
    assert!(output.status.success());
    // Fixture enables JSON globally; exercise the human-readable branch separately.
    let output = Command::new(env!("CARGO_BIN_EXE_waypoints"))
        .arg("--database")
        .arg(&fixture.database)
        .arg("--socket")
        .arg(&fixture.socket)
        .args([
            "node",
            "refresh",
            "--workflow",
            &workflow,
            "--node",
            &node_id,
        ])
        .output()?;
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("jj status: missing"));
    assert_eq!(jj(&project, &op_args)?, operation);
    fixture.stop()?;
    assert_eq!(
        fixture.success(&[
            "node",
            "refresh",
            "--workflow",
            &workflow,
            "--node",
            &node_id
        ])?,
        missing
    );
    fixture.stop()?;
    Ok(())
}

#[test]
fn branching_graph_survives_restart_and_rejects_invalid_edits() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let project = fixture
        .directory
        .path()
        .to_str()
        .expect("temporary path is UTF-8");
    let first = fixture.success(&[
        "workflow",
        "create",
        "--project",
        project,
        "--name",
        "First",
    ])?;
    let second = fixture.success(&[
        "workflow",
        "create",
        "--project",
        project,
        "--name",
        "Second",
    ])?;
    let workflow = first["id"].to_string();
    let other = second["id"].to_string();
    let root = fixture.success(&[
        "node",
        "create",
        "--workflow",
        &workflow,
        "--prompt",
        "Interfaces + placeholders",
    ])?;
    let source = root["id"].to_string();
    let mut children = Vec::new();
    let mut connections = Vec::new();
    for prompt in ["Authentication", "Image uploading", "Settings"] {
        let child = fixture.success(&[
            "node",
            "create",
            "--workflow",
            &workflow,
            "--prompt",
            prompt,
        ])?;
        let target = child["id"].to_string();
        connections.push(fixture.success(&[
            "connection",
            "create",
            "--workflow",
            &workflow,
            "--source",
            &source,
            "--target",
            &target,
        ])?);
        children.push(child);
    }
    let target = children[0]["id"].to_string();
    let prompt = "  Authentication\nUse bound SQL: '); DROP TABLE nodes; --  ";
    let updated = fixture.success(&[
        "node",
        "update",
        "--workflow",
        &workflow,
        "--node",
        &target,
        "--prompt",
        prompt,
    ])?;
    assert_eq!(updated["prompt"], prompt);
    assert_eq!(updated["id"], children[0]["id"]);
    assert_eq!(
        fixture.success(&["node", "get", "--workflow", &workflow, "--node", &target,])?,
        updated
    );
    let other_node = fixture.success(&[
        "node",
        "create",
        "--workflow",
        &other,
        "--prompt",
        "Separate",
    ])?;
    let foreign = other_node["id"].to_string();
    let graph = fixture.success(&["workflow", "graph", "--workflow", &workflow])?;
    assert_eq!(
        graph,
        json!({
            "workflow": first,
            "nodes": [root, updated, children[1], children[2]],
            "connections": connections,
        })
    );
    let other_graph = fixture.success(&["workflow", "graph", "--workflow", &other])?;
    assert_eq!(
        other_graph,
        json!({"workflow": second, "nodes": [other_node], "connections": []})
    );

    let invalid_commands = [
        vec![
            "node",
            "create",
            "--workflow",
            &workflow,
            "--prompt",
            " \t\n",
        ],
        vec![
            "node",
            "update",
            "--workflow",
            &workflow,
            "--node",
            &target,
            "--prompt",
            " ",
        ],
        vec![
            "node",
            "update",
            "--workflow",
            &workflow,
            "--node",
            &foreign,
            "--prompt",
            "Wrong",
        ],
        vec!["node", "get", "--workflow", &other, "--node", &source],
        vec!["workflow", "graph", "--workflow", "999999"],
        vec![
            "connection",
            "create",
            "--workflow",
            &workflow,
            "--source",
            &source,
            "--target",
            &foreign,
        ],
        vec![
            "connection",
            "create",
            "--workflow",
            &workflow,
            "--source",
            &source,
            "--target",
            &source,
        ],
        vec![
            "connection",
            "create",
            "--workflow",
            &workflow,
            "--source",
            &source,
            "--target",
            &target,
        ],
        vec![
            "connection",
            "create",
            "--workflow",
            &workflow,
            "--source",
            &target,
            "--target",
            &source,
        ],
    ];
    for args in invalid_commands {
        let output = fixture.command().args(&args).output()?;
        assert!(
            !output.status.success(),
            "invalid operation succeeded: {args:?}"
        );
        assert!(
            output.stdout.is_empty(),
            "errors must not contaminate JSON stdout"
        );
        assert!(
            !output.stderr.is_empty(),
            "invalid operation must explain failure"
        );
    }
    assert_eq!(
        fixture.success(&["workflow", "graph", "--workflow", &workflow])?,
        graph
    );
    assert_eq!(
        fixture.success(&["node", "get", "--workflow", &other, "--node", &foreign])?,
        other_node
    );
    for (method, params) in [
        ("node.create", json!({"workflow_id": first["id"]})),
        (
            "node.get",
            json!({"workflow_id": first["id"], "node_id": "bad"}),
        ),
        (
            "node.update",
            json!({"workflow_id": first["id"], "node_id": root["id"], "prompt": null}),
        ),
        (
            "connection.create",
            json!({"workflow_id": first["id"], "source_id": root["id"]}),
        ),
        (
            "workflow.graph",
            json!({"workflow_id": first["id"], "unexpected": true}),
        ),
    ] {
        let mut request = serde_json::to_vec(&json!({
            "jsonrpc": "2.0", "id": 1, "method": method, "params": params,
        }))?;
        request.push(b'\n');
        assert_eq!(fixture.rpc(&request)?["error"]["code"], -32602, "{method}");
    }
    fixture.stop()?;
    assert_eq!(
        fixture.success(&["workflow", "graph", "--workflow", &workflow])?,
        graph
    );
    assert_eq!(
        fixture.success(&["node", "get", "--workflow", &other, "--node", &foreign])?,
        other_node
    );
    fixture.stop()?;
    Ok(())
}

impl Drop for OwnedServer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn cli_autostarts_reuses_and_restarts_persistent_backend() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let mut create = fixture.command();
    let output = create
        .args([
            "workflow",
            "create",
            "--name",
            "First workflow",
            "--project",
        ])
        .arg(fixture.directory.path())
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let saved: Value = serde_json::from_slice(&output.stdout)?;
    let first = fixture.success(&["backend", "status"])?;
    let list: Value = serde_json::from_slice(&fixture.list_command().output()?.stdout)?;
    assert_eq!(list, json!([saved.clone()]));
    assert_eq!(first, fixture.success(&["backend", "status"])?);
    assert_eq!(
        fs::metadata(&fixture.socket)?.permissions().mode() & 0o777,
        0o600
    );

    fixture.stop()?;
    let output = fixture.list_command().output()?;
    assert!(output.status.success(), "restart must succeed");
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout)?,
        json!([saved])
    );
    assert_ne!(
        first["pid"],
        fixture.success(&["backend", "status"])?["pid"]
    );
    fixture.stop()?;
    Ok(())
}

#[test]
fn simultaneous_clients_start_only_one_backend() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let mut children = Vec::new();
    for _ in 0..8 {
        children.push(
            fixture
                .list_command()
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?,
        );
    }
    for child in children {
        let output = child.wait_with_output()?;
        assert!(
            output.status.success(),
            "concurrent starter failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(serde_json::from_slice::<Value>(&output.stdout)?, json!([]));
    }
    let first = fixture.success(&["backend", "status"])?;
    assert_eq!(first, fixture.success(&["backend", "status"])?);

    // Another socket cannot bypass the lock protecting this database.
    let output = fixture
        .command_at(&fixture.directory.path().join("other.sock"))
        .arg("serve")
        .output()?;
    assert!(
        !output.status.success(),
        "second database owner must be rejected"
    );
    assert_eq!(first, fixture.success(&["backend", "status"])?);
    Ok(())
}

#[test]
fn malformed_requests_and_invalid_input_do_not_stop_backend() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    fixture.start()?;
    assert_eq!(fixture.rpc(b"not json\n")?["error"]["code"], -32700);
    assert_eq!(fixture.rpc(b"{}\n")?["error"]["code"], -32600);
    assert_eq!(
        fixture.rpc(b"{\"jsonrpc\":\"2.0\",\"id\":\"test\",\"method\":\"missing\"}\n")?,
        json!({"jsonrpc":"2.0","id":"test","error":{"code":-32601,"message":"method not found"}})
    );
    assert_eq!(
        fixture.rpc(
            b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"workflow.create\",\"params\":{}}\n"
        )?["error"]["code"],
        -32602
    );
    let output = fixture
        .command()
        .args(["workflow", "create", "--name", " ", "--project"])
        .arg(fixture.directory.path())
        .output()?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("name must not be blank"));
    let output = fixture
        .command()
        .args(["workflow", "list", "--project"])
        .arg(fixture.directory.path().join("missing"))
        .output()?;
    assert!(!output.status.success());
    fixture.success(&["backend", "status"])?;
    Ok(())
}

#[test]
fn stale_socket_is_recovered_but_regular_files_are_preserved() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    drop(UnixListener::bind(&fixture.socket)?);
    fixture.start()?;
    fixture.stop()?;

    fs::write(&fixture.socket, "do not delete")?;
    let output = fixture.command().arg("serve").output()?;
    assert!(!output.status.success());
    assert_eq!(fs::read_to_string(&fixture.socket)?, "do not delete");
    Ok(())
}

#[test]
fn crashed_backend_releases_locks_and_can_be_restarted() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let mut server = OwnedServer(
        fixture
            .command()
            .arg("serve")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?,
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if fixture
            .command()
            .args(["backend", "status"])
            .output()?
            .status
            .success()
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "foreground backend did not become ready"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    server.0.kill()?;
    server.0.wait()?;
    assert!(
        fixture.socket.exists(),
        "an abrupt exit leaves a stale socket"
    );
    fixture.start()?;
    fixture.stop()?;
    Ok(())
}

#[test]
fn live_unresponsive_listener_is_not_replaced() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let listener = UnixListener::bind(&fixture.socket)?;
    let server = std::thread::spawn(move || -> std::io::Result<()> {
        let (mut stream, _) = listener.accept()?;
        stream.set_read_timeout(Some(Duration::from_secs(7)))?;
        let mut request = String::new();
        BufReader::new(&mut stream).read_line(&mut request)?;
        // The client's readiness deadline must expire without spawning another server.
        std::thread::sleep(Duration::from_millis(5500));
        Ok(())
    });
    let output = fixture.list_command().output()?;
    server.join().expect("test listener panicked")?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("timed out"));
    assert!(
        !fixture.database.exists(),
        "a live endpoint must not trigger database startup"
    );
    assert!(!fixture.directory.path().join("backend.sock.log").exists());
    Ok(())
}

#[test]
fn mismatched_database_is_rejected_without_replacing_backend() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    fixture.start()?;
    let output = Command::new(env!("CARGO_BIN_EXE_waypoints"))
        .arg("--database")
        .arg(fixture.directory.path().join("different.db"))
        .arg("--socket")
        .arg(&fixture.socket)
        .args(["workflow", "list", "--project"])
        .arg(fixture.directory.path())
        .output()?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("different database"));
    fixture.success(&["backend", "status"])?;
    Ok(())
}

#[test]
fn status_does_not_start_a_backend_and_help_needs_no_configuration() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let output: Output = fixture.command().args(["backend", "status"]).output()?;
    assert!(!output.status.success());
    assert!(!fixture.socket.exists());
    assert!(!fixture.database.exists());
    assert!(
        Command::new(env!("CARGO_BIN_EXE_waypoints"))
            .arg("--help")
            .env_remove("HOME")
            .env_remove("XDG_STATE_HOME")
            .status()?
            .success()
    );
    Ok(())
}
