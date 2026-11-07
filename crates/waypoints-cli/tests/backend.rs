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
use waypoints_desktop_ipc::{Request, Response};

#[path = "backend/archive.rs"]
mod archive;
#[path = "backend/discovery.rs"]
mod discovery;
#[path = "backend/frontend_backend.rs"]
mod frontend_backend;
#[path = "backend/multiple_parents.rs"]
mod multiple_parents;
#[path = "backend/planning.rs"]
mod planning;
#[path = "backend/rename.rs"]
mod rename;
#[path = "backend/restart.rs"]
mod restart;
#[path = "backend/restore.rs"]
mod restore;
#[path = "backend/restore_downstream.rs"]
mod restore_downstream;
#[path = "backend/restore_impact.rs"]
mod restore_impact;
#[path = "backend/revisions.rs"]
mod revisions;
#[path = "backend/upstream.rs"]
mod upstream;
#[path = "backend/workspace_deletion.rs"]
mod workspace_deletion;

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
        self.json_output(output, &format!("CLI {args:?}"))
    }

    fn json_output(&self, output: Output, context: &str) -> anyhow::Result<Value> {
        assert!(
            output.status.success(),
            "{context} failed ({}):\nstdout: {}\nstderr: {}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
            self.startup_diagnostics()
        );
        Ok(serde_json::from_slice(&output.stdout)?)
    }

    fn startup_diagnostics(&self) -> String {
        let log = self.directory.path().join("backend.sock.log");
        format!(
            "fixture: {}\nsocket metadata: {:?}\nstartup log:\n{}",
            self.directory.path().display(),
            fs::symlink_metadata(&self.socket),
            fs::read_to_string(&log).unwrap_or_else(|error| format!("{}: {error}", log.display()))
        )
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
        self.json_output(output, "startup")?;
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
        if std::thread::panicking() {
            // TempDir removes the log after this drop; retain it in test output
            // before cleanup can change the evidence.
            eprintln!("{}", self.startup_diagnostics());
        }
        // Clean up the real background process even when an assertion unwinds.
        let _ = self.command().args(["backend", "stop"]).output();
    }
}

struct OwnedServer(Child);

fn run_fixture(prompt: &str) -> anyhow::Result<(Fixture, PathBuf, PathBuf)> {
    let fixture = Fixture::new()?;
    jj(fixture.directory.path(), &["git", "init", "project"])?;
    let project = fixture.directory.path().join("project");
    fs::write(project.join("baseline.txt"), "original\n")?;
    jj(&project, &["describe", "-m", "Run baseline"])?;
    fixture.success(&[
        "workflow",
        "create",
        "--project",
        project.to_str().expect("project path"),
        "--name",
        "Runs",
    ])?;
    fixture.success(&["node", "create", "--workflow", "1", "--prompt", prompt])?;
    fixture.success(&[
        "node",
        "create-change",
        "--workflow",
        "1",
        "--node",
        "1",
        "--base",
        "root()",
    ])?;
    let workspace = fixture.directory.path().join("agent-workspace");
    fixture.success(&[
        "node",
        "workspace",
        "create",
        "--workflow",
        "1",
        "--node",
        "1",
        "--directory",
        workspace.to_str().expect("workspace path"),
    ])?;
    let agent = fixture.directory.path().join("fake-acp");
    fs::write(&agent, include_str!("fixtures/acp_agent.py"))?;
    fs::set_permissions(&agent, fs::Permissions::from_mode(0o700))?;
    Ok((fixture, agent, workspace))
}

fn start_run(fixture: &Fixture, agent: &Path) -> anyhow::Result<Value> {
    fixture.success(&[
        "run",
        "start",
        "--workflow",
        "1",
        "--node",
        "1",
        "--executable",
        agent.to_str().expect("agent path"),
    ])
}

fn continue_run(fixture: &Fixture, id: &str, prompt: &str) -> anyhow::Result<Value> {
    fixture.success(&["run", "continue", "--run", id, "--prompt", prompt])
}

fn agent_requests(agent: &Path) -> anyhow::Result<Vec<Value>> {
    fs::read_to_string(agent.with_extension("requests.jsonl"))?
        .lines()
        .map(|line| Ok(serde_json::from_str(line)?))
        .collect()
}

fn intercept_snapshots(fixture: &Fixture, script: &str) -> anyhow::Result<()> {
    fixture.stop()?;
    let native = Command::new("/bin/sh")
        .args(["-c", "command -v jj"])
        .output()?;
    assert!(native.status.success());
    let native = String::from_utf8(native.stdout)?.trim().to_owned();
    let bin = fixture.directory.path().join("snapshot-bin");
    fs::create_dir(&bin)?;
    let wrapper = bin.join("jj");
    fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nnative='{native}'\nfor arg do\n if [ \"$arg\" = snapshot.auto-update-stale=false ]; then\n{script}\n fi\ndone\nexec \"$native\" \"$@\"\n"
        ),
    )?;
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700))?;
    let path = format!("{}:{}", bin.display(), std::env::var("PATH")?);
    let output = fixture.list_command().env("PATH", path).output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

fn wait_for_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() {
        assert!(Instant::now() < deadline, "file never appeared: {path:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn crash_fixture_backend(fixture: &Fixture) -> anyhow::Result<()> {
    let pid = fixture.success(&["backend", "status"])?["pid"].to_string();
    assert!(
        Command::new("/bin/kill")
            .args(["-KILL", &pid])
            .status()?
            .success()
    );
    Ok(())
}

fn stop_fixture_agent_group(workspace: &Path) -> anyhow::Result<()> {
    // Peers without descendants exit on stdin EOF when their backend dies.
    // Do not signal an already-exited adapter's former process group.
    if workspace.join("descendant.pid").exists() {
        // This fixture's live child retains its freshly launched process group.
        let pid = fs::read_to_string(workspace.join("adapter.pid"))?;
        let _ = Command::new("/bin/kill")
            .args(["-KILL", "--", &format!("-{}", pid.trim())])
            .status()?;
        assert_process_stopped(&workspace.join("descendant.pid"))?;
    }
    assert_process_stopped(&workspace.join("adapter.pid"))
}

fn recover_run(fixture: &Fixture, id: &str) -> anyhow::Result<Value> {
    fixture.success(&["run", "recover", "--run", id, "--confirm-processes-stopped"])
}

fn wait_run(fixture: &Fixture, id: &str, state: &str) -> anyhow::Result<Value> {
    wait_run_for(fixture, id, state, Duration::from_secs(10))
}

fn wait_run_for(
    fixture: &Fixture,
    id: &str,
    state: &str,
    duration: Duration,
) -> anyhow::Result<Value> {
    let deadline = Instant::now() + duration;
    loop {
        let status = fixture.success(&["run", "status", "--run", id])?;
        if status["run"]["state"]["status"] == state {
            return Ok(status);
        }
        assert!(Instant::now() < deadline, "expected {state}, got {status}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn assert_process_stopped(pid: &Path) -> anyhow::Result<()> {
    let pid = fs::read_to_string(pid)?;
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let output = Command::new("ps")
            .args(["-p", &pid, "-o", "stat="])
            .output()?;
        // Orphan descendants may briefly remain as zombies before the OS reaps them.
        let state = String::from_utf8(output.stdout)?;
        if !output.status.success() || state.trim().starts_with('Z') {
            return Ok(());
        }
        assert!(
            Instant::now() < deadline,
            "process {pid} still running: {state}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn continuation_loads_harness_history_after_restart_without_replacing_node_assignment()
-> anyhow::Result<()> {
    let assignment = "remember conversation-only-token-a37c";
    let (fixture, agent, workspace) = run_fixture(assignment)?;
    let first = start_run(&fixture, &agent)?;
    let id = first["run"]["id"].to_string();
    let completed = wait_run(&fixture, &id, "completed")?;
    assert!(completed["run"]["continued_from"].is_null());
    assert_process_stopped(&workspace.join("adapter.pid"))?;
    for entry in fs::read_dir(&workspace)? {
        let path = entry?.path();
        if path.is_file() {
            assert!(
                !fs::read_to_string(&path)?.contains("conversation-only-token-a37c"),
                "{path:?}"
            );
        }
    }
    fixture.stop()?;
    // Continuation itself auto-starts the backend; neither adapter nor event ring survives.
    let second = continue_run(&fixture, &id, "recall and extend")?;
    let second_id = second["run"]["id"].to_string();
    assert_ne!(second_id, id);
    assert_eq!(second["run"]["continued_from"], first["run"]["id"]);
    assert_eq!(second["run"]["prompt"], "recall and extend");
    assert_eq!(second["run"]["session_id"], completed["run"]["session_id"]);
    assert_eq!(second["run"]["change_id"], completed["run"]["change_id"]);
    assert_eq!(second["run"]["directory"], completed["run"]["directory"]);
    let second_done = wait_run(&fixture, &second_id, "completed")?;
    assert_process_stopped(&workspace.join("adapter.pid"))?;
    assert_eq!(
        fs::read_to_string(workspace.join("hello.txt"))?,
        "Hello from the first turn.\nExtended in the second turn.\n"
    );
    assert_eq!(
        fixture.success(&["node", "get", "--workflow", "1", "--node", "1"])?["prompt"],
        assignment
    );
    assert_eq!(
        fixture.success(&["run", "status", "--run", &id])?,
        completed
    );
    let events = fixture.success(&["run", "events", "--run", &second_id])?;
    let items = events["events"].as_array().expect("events");
    assert_eq!(items.len(), 3, "{events}");
    assert_eq!(items[0]["origin"], "replay");
    assert_eq!(items[1]["origin"], "replay");
    assert_eq!(items[1]["update"]["toolCallId"], "historical-edit");
    assert_eq!(items[2]["origin"], "live");
    assert_eq!(
        items[2]["update"]["content"]["text"],
        "Recalled: conversation-only-token-a37c"
    );
    let cursor = events["next"].to_string();
    assert_eq!(
        fixture.success(&["run", "events", "--run", &second_id, "--after", &cursor])?["events"],
        json!([])
    );
    let requests = agent_requests(&agent)?;
    let methods: Vec<_> = requests
        .iter()
        .map(|request| request["method"].as_str().expect("method"))
        .collect();
    assert_eq!(
        methods,
        [
            "initialize",
            "session/new",
            "session/set_mode",
            "session/prompt",
            "initialize",
            "session/load",
            "session/set_mode",
            "session/prompt"
        ]
    );
    // Completion includes an automatic snapshot, without a client-side native mutation.
    assert_eq!(
        completed["run"]["snapshot"]["status"], "saved",
        "{completed}"
    );
    assert_eq!(
        second_done["run"]["snapshot"]["status"], "saved",
        "{second_done}"
    );
    let first_revision = &completed["run"]["snapshot"]["revision"];
    let second_revision = &second_done["run"]["snapshot"]["revision"];
    assert_eq!(first_revision["change_id"], second_revision["change_id"]);
    assert_ne!(first_revision["commit_id"], second_revision["commit_id"]);
    assert_eq!(
        jj(
            &workspace,
            &[
                "--ignore-working-copy",
                "--at-op=@",
                "file",
                "show",
                "-r",
                first_revision["commit_id"].as_str().expect("commit"),
                "hello.txt"
            ]
        )?,
        "Hello from the first turn.\n"
    );
    assert_eq!(
        jj(
            &workspace,
            &[
                "--ignore-working-copy",
                "--at-op=@",
                "file",
                "show",
                "-r",
                second_revision["commit_id"].as_str().expect("commit"),
                "hello.txt"
            ]
        )?,
        "Hello from the first turn.\nExtended in the second turn.\n"
    );
    let refreshed = fixture.success(&["node", "refresh", "--workflow", "1", "--node", "1"])?;
    assert_eq!(
        refreshed["association"]["change_id"],
        completed["run"]["change_id"]
    );
    fixture.stop()?;
    assert_eq!(
        fixture.success(&["run", "status", "--run", &second_id])?,
        second_done
    );
    Ok(())
}

#[test]
fn recovery_requires_confirmation_preserves_work_and_allows_explicit_continuation()
-> anyhow::Result<()> {
    let (fixture, agent, workspace) = run_fixture("hang")?;
    fs::write(workspace.join("keep.txt"), "preserve dirty work\n")?;
    let started = start_run(&fixture, &agent)?;
    let id = started["run"]["id"].to_string();
    wait_run(&fixture, &id, "running")?;
    wait_for_file(&workspace.join("descendant.pid"));
    crash_fixture_backend(&fixture)?;
    let interrupted = fixture.success(&["run", "status", "--run", &id])?;
    assert_eq!(interrupted["run"]["state"]["status"], "interrupted");
    assert_eq!(interrupted["run"]["state"]["phase"], "execution");
    let before_commit = recorded_commit(&workspace)?;
    let before_op = jj(
        &workspace,
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
    let checkout = fs::read(workspace.join(".jj/working_copy/checkout"))?;
    let tree = fs::read(workspace.join(".jj/working_copy/tree_state"))?;
    let requests = agent_requests(&agent)?;
    let inspection = fixture.success(&["run", "recovery", "--run", &id])?;
    assert_eq!(inspection["run"], interrupted["run"]);
    assert_eq!(inspection["workspace"]["status"], "verified");
    // Workspace verification is not evidence old processes stopped.
    let descendant = fs::read_to_string(workspace.join("descendant.pid"))?;
    assert!(
        Command::new("ps")
            .args(["-p", descendant.trim(), "-o", "stat="])
            .output()?
            .status
            .success()
    );
    let omitted = fixture
        .command()
        .args(["run", "recover", "--run", &id])
        .output()?;
    assert!(!omitted.status.success());
    assert!(String::from_utf8(omitted.stderr)?.contains("--confirm-processes-stopped"));
    for params in [
        json!({"run_id":1}),
        json!({"run_id":1,"confirmation":false}),
        json!({"run_id":1,"confirmation":"unchecked"}),
        json!({"run_id":1,"confirmation":"processes_stopped","force":true}),
    ] {
        let response = fixture.rpc(
            format!(
                "{}\n",
                json!({"jsonrpc":"2.0","id":1,"method":"run.recover","params":params})
            )
            .as_bytes(),
        )?;
        assert_eq!(response["error"]["code"], -32602);
    }
    assert_eq!(
        fixture.success(&["run", "status", "--run", &id])?,
        interrupted
    );
    fixture.stop()?;
    // Repeated backend restarts retain the first interruption's phase/cause.
    assert_eq!(
        fixture.success(&["run", "status", "--run", &id])?,
        interrupted
    );
    stop_fixture_agent_group(&workspace)?;
    assert_process_stopped(&workspace.join("descendant.pid"))?;
    let recovered = recover_run(&fixture, &id)?;
    assert_eq!(recovered["run"]["state"]["status"], "recovered");
    assert_eq!(
        recovered["run"]["state"]["detail"],
        interrupted["run"]["state"]["detail"]
    );
    assert_eq!(recovered["run"]["state"]["phase"], "execution");
    for field in [
        "session_id",
        "process_id",
        "directory",
        "change_id",
        "snapshot",
        "prompt",
    ] {
        assert_eq!(
            recovered["run"][field], interrupted["run"][field],
            "{field}"
        );
    }
    assert_eq!(
        fs::read_to_string(workspace.join("keep.txt"))?,
        "preserve dirty work\n"
    );
    assert_eq!(
        fs::read(workspace.join(".jj/working_copy/checkout"))?,
        checkout
    );
    assert_eq!(
        fs::read(workspace.join(".jj/working_copy/tree_state"))?,
        tree
    );
    assert_eq!(recorded_commit(&workspace)?, before_commit);
    assert_eq!(
        jj(
            &workspace,
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
        before_op
    );
    assert_eq!(
        agent_requests(&agent)?,
        requests,
        "recovery must not replay anything"
    );
    fixture.stop()?;
    assert_eq!(
        fixture.success(&["run", "status", "--run", &id])?,
        recovered
    );
    let next = continue_run(&fixture, &id, "permission")?;
    let next_id = next["run"]["id"].to_string();
    wait_run(&fixture, &next_id, "awaiting_permission")?;
    // Retrying an old recovery must not release the new run's reservation.
    assert_eq!(recover_run(&fixture, &id)?, recovered);
    assert!(
        !fixture
            .command()
            .args([
                "run",
                "start",
                "--workflow",
                "1",
                "--node",
                "1",
                "--executable",
                agent.to_str().expect("agent")
            ])
            .output()?
            .status
            .success()
    );
    fixture.success(&[
        "run",
        "permission",
        "--run",
        &next_id,
        "--permission",
        "1",
        "--option",
        "allow",
    ])?;
    let next_done = wait_run(&fixture, &next_id, "completed")?;
    assert_eq!(next_done["run"]["snapshot"]["status"], "saved");
    assert_eq!(
        fixture.success(&["run", "status", "--run", &id])?,
        recovered,
        "later work must not fill the interrupted run's snapshot"
    );
    Ok(())
}

#[test]
fn recovery_rechecks_workspace_and_refuses_missing_switched_or_stale_checkout() -> anyhow::Result<()>
{
    for failure in ["missing", "switched", "stale"] {
        let (fixture, agent, workspace) = run_fixture("permission")?;
        let started = start_run(&fixture, &agent)?;
        let id = started["run"]["id"].to_string();
        wait_run(&fixture, &id, "awaiting_permission")?;
        crash_fixture_backend(&fixture)?;
        stop_fixture_agent_group(&workspace)?;
        let interrupted = fixture.success(&["run", "status", "--run", &id])?;
        assert_eq!(
            fixture.success(&["run", "recovery", "--run", &id])?["workspace"]["status"],
            "verified"
        );
        let moved = fixture.directory.path().join("moved-workspace");
        match failure {
            "missing" => fs::rename(&workspace, &moved)?,
            "switched" => {
                jj(&workspace, &["new", "-m", "Externally switched"])?;
            }
            "stale" => {
                let project = fixture.directory.path().join("project");
                jj(
                    &project,
                    &[
                        "--ignore-working-copy",
                        "describe",
                        "-r",
                        interrupted["run"]["change_id"].as_str().expect("change"),
                        "-m",
                        "Rewritten outside workspace",
                    ],
                )?;
            }
            _ => unreachable!("known failure"),
        }
        let directory = if failure == "missing" {
            &moved
        } else {
            &workspace
        };
        let checkout = fs::read(directory.join(".jj/working_copy/checkout"))?;
        let inspection = fixture.success(&["run", "recovery", "--run", &id])?;
        assert_eq!(
            inspection["workspace"]["status"], "blocked",
            "{failure}: {inspection}"
        );
        assert!(
            !fixture
                .command()
                .args([
                    "run",
                    "recover",
                    "--run",
                    &id,
                    "--confirm-processes-stopped"
                ])
                .output()?
                .status
                .success()
        );
        assert_eq!(
            fixture.success(&["run", "status", "--run", &id])?,
            interrupted
        );
        assert_eq!(
            fs::read(directory.join(".jj/working_copy/checkout"))?,
            checkout
        );
        assert!(
            !fixture
                .command()
                .args([
                    "run",
                    "start",
                    "--workflow",
                    "1",
                    "--node",
                    "1",
                    "--executable",
                    agent.to_str().expect("agent")
                ])
                .output()?
                .status
                .success(),
            "failed recovery must retain reservation"
        );
    }
    Ok(())
}

#[test]
fn recovery_without_a_session_allows_fresh_start_but_never_invents_continuation()
-> anyhow::Result<()> {
    let (fixture, agent, workspace) = run_fixture("complete")?;
    fs::write(workspace.join("stall-initialize"), "")?;
    let started = start_run(&fixture, &agent)?;
    let id = started["run"]["id"].to_string();
    wait_for_file(&workspace.join("adapter.pid"));
    crash_fixture_backend(&fixture)?;
    stop_fixture_agent_group(&workspace)?;
    fixture.success(&["run", "recovery", "--run", &id])?;
    let recovered = recover_run(&fixture, &id)?;
    assert!(recovered["run"]["session_id"].is_null());
    assert!(
        !fixture
            .command()
            .args(["run", "continue", "--run", &id, "--prompt", "complete"])
            .output()?
            .status
            .success()
    );
    fs::remove_file(workspace.join("stall-initialize"))?;
    let next = start_run(&fixture, &agent)?;
    wait_run(&fixture, &next["run"]["id"].to_string(), "completed")?;
    assert_eq!(recover_run(&fixture, &id)?, recovered);
    Ok(())
}

#[test]
fn recovery_refuses_active_and_successful_runs_and_inspection_does_not_release()
-> anyhow::Result<()> {
    let (fixture, agent, _) = run_fixture("permission")?;
    let started = start_run(&fixture, &agent)?;
    let id = started["run"]["id"].to_string();
    wait_run(&fixture, &id, "awaiting_permission")?;
    for command in ["recovery", "recover"] {
        let mut args = vec!["run", command, "--run", &id];
        if command == "recover" {
            args.push("--confirm-processes-stopped");
        }
        assert!(!fixture.command().args(args).output()?.status.success());
    }
    assert_eq!(
        fixture.success(&["run", "status", "--run", &id])?["run"]["state"]["status"],
        "awaiting_permission"
    );
    fixture.success(&[
        "run",
        "permission",
        "--run",
        &id,
        "--permission",
        "1",
        "--option",
        "allow",
    ])?;
    let completed = wait_run(&fixture, &id, "completed")?;
    assert!(
        !fixture
            .command()
            .args([
                "run",
                "recover",
                "--run",
                &id,
                "--confirm-processes-stopped"
            ])
            .output()?
            .status
            .success()
    );
    assert_eq!(
        fixture.success(&["run", "status", "--run", &id])?,
        completed
    );
    Ok(())
}

fn recorded_commit(workspace: &Path) -> anyhow::Result<String> {
    jj(
        workspace,
        &[
            "--ignore-working-copy",
            "--at-op=@",
            "log",
            "--no-graph",
            "-r",
            "@",
            "-T",
            "commit_id",
        ],
    )
}

#[test]
fn snapshots_record_exact_revisions_without_client_mutation_and_do_not_repeat() -> anyhow::Result<()>
{
    let (fixture, agent, workspace) = run_fixture("complete")?;
    let project = fixture.directory.path().join("project");
    let source_commit = recorded_commit(&project)?;
    let before = recorded_commit(&workspace)?;
    let started = start_run(&fixture, &agent)?;
    let id = started["run"]["id"].to_string();
    let completed = wait_run(&fixture, &id, "completed")?;
    let snapshot = &completed["run"]["snapshot"];
    assert_eq!(snapshot["status"], "saved", "{completed}");
    assert_eq!(
        snapshot["revision"]["change_id"],
        completed["run"]["change_id"]
    );
    let commit = snapshot["revision"]["commit_id"]
        .as_str()
        .expect("saved commit");
    assert_ne!(commit, before);
    assert_eq!(recorded_commit(&workspace)?, commit);
    assert_eq!(
        jj(
            &workspace,
            &[
                "--ignore-working-copy",
                "--at-op=@",
                "file",
                "show",
                "-r",
                commit,
                "captured-prompt.txt"
            ]
        )?,
        "complete"
    );
    assert_eq!(recorded_commit(&project)?, source_commit);
    assert_process_stopped(&workspace.join("adapter.pid"))?;
    fs::write(
        workspace.join("after-run.txt"),
        "manual edits are not part of that run\n",
    )?;
    fixture.success(&["run", "events", "--run", &id])?;
    let refreshed = fixture.success(&["node", "refresh", "--workflow", "1", "--node", "1"])?;
    assert_eq!(refreshed["state"]["revision"], snapshot["revision"]);
    fixture.stop()?;
    assert_eq!(
        fixture.success(&["run", "status", "--run", &id])?,
        completed
    );
    assert_eq!(recorded_commit(&workspace)?, commit);
    Ok(())
}

#[test]
fn reviews_use_exact_saved_commits_even_after_rewrites_and_workspace_removal() -> anyhow::Result<()>
{
    let (fixture, agent, workspace) = run_fixture("complete")?;
    let project = fixture.directory.path().join("project");
    let started = start_run(&fixture, &agent)?;
    let id = started["run"]["id"].to_string();
    let completed = wait_run(&fixture, &id, "completed")?;
    let saved = &completed["run"]["snapshot"]["revision"];
    let original = fixture.success(&["run", "review", "--run", &id])?;
    assert_eq!(&original["revision"], saved);
    assert!(
        original["summary"]
            .as_str()
            .expect("summary")
            .contains("captured-prompt.txt")
    );
    assert!(
        original["patch"]
            .as_str()
            .expect("patch")
            .contains("+complete")
    );
    assert_eq!(
        fixture.success(&["node", "review", "--workflow", "1", "--node", "1"])?,
        original
    );
    fs::write(workspace.join("later.txt"), "later recorded code\n")?;
    jj(&workspace, &["describe", "-m", "A later revision"])?;
    fs::write(workspace.join("dirty.txt"), "must not snapshot\n")?;
    let operation = jj(
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
    let current = fixture.success(&["node", "review", "--workflow", "1", "--node", "1"])?;
    assert_ne!(current["revision"]["commit_id"], saved["commit_id"]);
    assert!(
        current["summary"]
            .as_str()
            .expect("summary")
            .contains("later.txt")
    );
    assert!(
        !current["summary"]
            .as_str()
            .expect("summary")
            .contains("dirty.txt")
    );
    assert_eq!(fixture.success(&["run", "review", "--run", &id])?, original);
    assert_eq!(
        fs::read_to_string(workspace.join("dirty.txt"))?,
        "must not snapshot\n"
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
        operation
    );
    for (method, params) in [
        (
            "node.review",
            json!({"workflow_id": 1, "node_id": 1, "revision": "@"}),
        ),
        ("run.review", json!({"run_id": 1, "snapshot": true})),
    ] {
        let mut bytes = serde_json::to_vec(
            &json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}),
        )?;
        bytes.push(b'\n');
        assert_eq!(fixture.rpc(&bytes)?["error"]["code"], -32602);
    }
    let human = Command::new(env!("CARGO_BIN_EXE_waypoints"))
        .arg("--database")
        .arg(&fixture.database)
        .arg("--socket")
        .arg(&fixture.socket)
        .args(["run", "review", "--run", &id])
        .output()?;
    assert!(human.status.success());
    assert!(String::from_utf8(human.stdout)?.contains("diff --git"));
    fixture.stop()?;
    // Only a disposable test-owned workspace is removed, not any user checkout.
    fs::remove_dir_all(&workspace)?;
    assert_eq!(fixture.success(&["run", "review", "--run", &id])?, original);
    Ok(())
}

#[test]
fn run_review_refuses_unsaved_snapshots_instead_of_using_current_code() -> anyhow::Result<()> {
    let (fixture, agent, _workspace) = run_fixture("sleep")?;
    let started = start_run(&fixture, &agent)?;
    let id = started["run"]["id"].to_string();
    assert!(started["run"]["upstream_inputs"].is_null());
    assert_eq!(
        fixture.success(&["run", "inputs", "--run", &id])?["status"],
        "unknown"
    );
    let output = fixture
        .command()
        .args(["run", "review", "--run", &id])
        .output()?;
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr)?.contains("no saved native snapshot"));
    fixture.success(&["run", "cancel", "--run", &id])?;
    wait_run(&fixture, &id, "cancelled")?;
    let output = fixture
        .command()
        .args(["run", "review", "--run", &id])
        .output()?;
    assert!(!output.status.success());
    Ok(())
}

#[test]
fn snapshot_failure_is_separate_from_agent_success_and_releases_reservation() -> anyhow::Result<()>
{
    let (fixture, agent, workspace) = run_fixture("complete")?;
    intercept_snapshots(&fixture, "echo 'injected snapshot failure' >&2\nexit 1")?;
    let before = recorded_commit(&workspace)?;
    let started = start_run(&fixture, &agent)?;
    let id = started["run"]["id"].to_string();
    let completed = wait_run(&fixture, &id, "completed")?;
    assert_eq!(
        completed["run"]["snapshot"]["status"], "failed",
        "{completed}"
    );
    assert!(
        completed["run"]["snapshot"]["detail"]
            .as_str()
            .expect("failure")
            .contains("injected snapshot failure")
    );
    assert_eq!(
        fs::read_to_string(workspace.join("captured-prompt.txt"))?,
        "complete"
    );
    assert_eq!(recorded_commit(&workspace)?, before);
    assert_process_stopped(&workspace.join("adapter.pid"))?;
    let next = continue_run(&fixture, &id, "complete")?;
    wait_run(&fixture, &next["run"]["id"].to_string(), "completed")?;
    assert_eq!(
        agent_requests(&agent)?
            .iter()
            .filter(|request| request["method"] == "session/prompt")
            .count(),
        2,
        "no automatic agent retry"
    );
    fixture.stop()?;
    assert_eq!(
        fixture.success(&["run", "status", "--run", &id])?,
        completed
    );
    Ok(())
}

#[test]
fn snapshot_warnings_do_not_silently_claim_all_output_was_recorded() -> anyhow::Result<()> {
    let (fixture, agent, workspace) = run_fixture("large-file")?;
    // Force a deterministic native new-file limit, independent of user config.
    intercept_snapshots(
        &fixture,
        "\"$native\" --config 'snapshot.max-new-file-size=\"1MiB\"' \"$@\"\nexit $?",
    )?;
    let started = start_run(&fixture, &agent)?;
    let id = started["run"]["id"].to_string();
    let completed = wait_run(&fixture, &id, "completed")?;
    assert_eq!(
        completed["run"]["snapshot"]["status"], "failed",
        "{completed}"
    );
    let detail = completed["run"]["snapshot"]["detail"]
        .as_str()
        .expect("failure");
    assert!(
        detail.contains("large.txt") && detail.contains("diagnostics"),
        "{detail}"
    );
    assert_eq!(
        fs::metadata(workspace.join("large.txt"))?.len(),
        2 * 1024 * 1024
    );
    Ok(())
}

#[test]
fn snapshot_decode_failure_does_not_claim_success_after_a_native_mutation() -> anyhow::Result<()> {
    let (fixture, agent, workspace) = run_fixture("complete")?;
    intercept_snapshots(
        &fixture,
        "\"$native\" \"$@\" > /dev/null || exit $?\nprintf 'invalid JSON\\n'\nexit 0",
    )?;
    let before = recorded_commit(&workspace)?;
    let started = start_run(&fixture, &agent)?;
    let id = started["run"]["id"].to_string();
    let completed = wait_run(&fixture, &id, "completed")?;
    assert_eq!(
        completed["run"]["snapshot"]["status"], "failed",
        "{completed}"
    );
    assert!(
        completed["run"]["snapshot"]["detail"]
            .as_str()
            .expect("failure")
            .contains("native snapshot may have succeeded")
    );
    assert_ne!(
        recorded_commit(&workspace)?,
        before,
        "native mutation actually happened"
    );
    assert_eq!(
        agent_requests(&agent)?
            .iter()
            .filter(|request| request["method"] == "session/prompt")
            .count(),
        1
    );
    fixture.stop()?;
    assert_eq!(
        fixture.success(&["run", "status", "--run", &id])?,
        completed
    );
    Ok(())
}

#[test]
fn crash_during_finalization_retains_reservation_without_repeating_snapshot() -> anyhow::Result<()>
{
    let (fixture, agent, workspace) = run_fixture("complete")?;
    let entered = fixture.directory.path().join("snapshot-entered");
    let release = fixture.directory.path().join("snapshot-release");
    let output = fixture.directory.path().join("snapshot-output");
    intercept_snapshots(
        &fixture,
        &format!(
            "\"$native\" \"$@\" > '{}' || exit $?\necho $$ > '{}'\nwhile [ ! -f '{}' ]; do /bin/sleep 0.05; done\ncat '{}'\nexit 0",
            output.display(),
            entered.display(),
            release.display(),
            output.display(),
        ),
    )?;
    let before = recorded_commit(&workspace)?;
    let started = start_run(&fixture, &agent)?;
    let id = started["run"]["id"].to_string();
    wait_for_file(&entered);
    wait_run(&fixture, &id, "finalizing")?;
    let unrecorded_commit = recorded_commit(&workspace)?;
    assert_ne!(
        unrecorded_commit, before,
        "native snapshot committed before the crash"
    );
    assert_process_stopped(&workspace.join("adapter.pid"))?;
    let backend_pid = fixture.success(&["backend", "status"])?["pid"].to_string();
    assert!(
        Command::new("/bin/kill")
            .args(["-KILL", &backend_pid])
            .status()?
            .success()
    );
    // Fixture-owned native wrapper only; never signal a PID recovered from storage.
    let native_pid = fs::read_to_string(&entered)?;
    let _ = Command::new("/bin/kill")
        .args(["-KILL", native_pid.trim()])
        .status()?;
    assert_process_stopped(&entered)?;
    fs::write(
        workspace.join("after-crash.txt"),
        "do not silently snapshot later edits\n",
    )?;
    let restarted = fixture.success(&["run", "status", "--run", &id])?;
    assert_eq!(restarted["run"]["state"]["status"], "interrupted");
    assert!(
        restarted["run"]["state"]["detail"]
            .as_str()
            .expect("detail")
            .contains("finalization")
    );
    assert_eq!(restarted["run"]["snapshot"]["status"], "not_recorded");
    assert_eq!(recorded_commit(&workspace)?, unrecorded_commit);
    assert!(
        !fixture
            .command()
            .args(["run", "continue", "--run", &id, "--prompt", "complete"])
            .output()?
            .status
            .success()
    );
    assert!(
        !fixture
            .command()
            .args([
                "run",
                "start",
                "--workflow",
                "1",
                "--node",
                "1",
                "--executable",
                agent.to_str().expect("agent")
            ])
            .output()?
            .status
            .success()
    );
    assert_eq!(
        agent_requests(&agent)?
            .iter()
            .filter(|request| request["method"] == "session/prompt")
            .count(),
        1
    );
    assert_eq!(restarted["run"]["state"]["phase"], "finalization");
    fixture.stop()?;
    assert_eq!(
        fixture.success(&["run", "status", "--run", &id])?,
        restarted
    );
    let inspection = fixture.success(&["run", "recovery", "--run", &id])?;
    assert_eq!(inspection["workspace"]["status"], "verified");
    let recovered = recover_run(&fixture, &id)?;
    assert_eq!(recovered["run"]["state"]["status"], "recovered");
    assert_eq!(recovered["run"]["state"]["phase"], "finalization");
    assert_eq!(recovered["run"]["snapshot"]["status"], "not_recorded");
    assert_eq!(
        recorded_commit(&workspace)?,
        unrecorded_commit,
        "recovery must not retry the lost snapshot"
    );
    Ok(())
}

#[test]
fn finalizing_retains_reservation_and_shutdown_waits_for_snapshot() -> anyhow::Result<()> {
    let (fixture, agent, workspace) = run_fixture("complete")?;
    let entered = fixture.directory.path().join("snapshot-entered");
    let release = fixture.directory.path().join("snapshot-release");
    intercept_snapshots(
        &fixture,
        &format!(
            "echo $$ > '{}'\nwhile [ ! -f '{}' ]; do /bin/sleep 0.05; done",
            entered.display(),
            release.display()
        ),
    )?;
    let started = start_run(&fixture, &agent)?;
    let id = started["run"]["id"].to_string();
    wait_for_file(&entered);
    let finalizing = wait_run(&fixture, &id, "finalizing")?;
    assert_eq!(finalizing["run"]["snapshot"]["status"], "not_recorded");
    assert_process_stopped(&workspace.join("adapter.pid"))?;
    fixture.success(&["backend", "status"])?;
    assert_eq!(
        fixture.success(&["run", "cancel", "--run", &id])?["run"]["state"]["status"],
        "finalizing",
        "finalization cannot be cancelled"
    );
    for args in [
        vec!["run", "continue", "--run", &id, "--prompt", "complete"],
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
        assert!(
            !fixture.command().args(args).output()?.status.success(),
            "workspace must remain reserved"
        );
    }
    fixture.success(&["backend", "stop"])?;
    // Shutdown acknowledges immediately, but keeps locks while snapshotting.
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        fixture.socket.exists(),
        "backend exited before native finalization"
    );
    fs::write(&release, "")?;
    let deadline = Instant::now() + Duration::from_secs(8);
    while fixture.socket.exists() {
        assert!(
            Instant::now() < deadline,
            "backend did not finish snapshot on shutdown"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let completed = fixture.success(&["run", "status", "--run", &id])?;
    assert_eq!(completed["run"]["state"]["status"], "completed");
    assert_eq!(
        completed["run"]["snapshot"]["status"], "saved",
        "{completed}"
    );
    Ok(())
}

#[test]
fn snapshot_preflight_refuses_switched_workspaces_and_cancelled_runs_do_not_snapshot()
-> anyhow::Result<()> {
    let (fixture, agent, workspace) = run_fixture("permission")?;
    let before = recorded_commit(&workspace)?;
    let started = start_run(&fixture, &agent)?;
    let id = started["run"]["id"].to_string();
    wait_run(&fixture, &id, "awaiting_permission")?;
    fixture.success(&["run", "cancel", "--run", &id])?;
    let cancelled = wait_run(&fixture, &id, "cancelled")?;
    assert_eq!(cancelled["run"]["snapshot"]["status"], "not_recorded");
    assert_eq!(recorded_commit(&workspace)?, before);
    let next = continue_run(&fixture, &id, "permission")?;
    let id = next["run"]["id"].to_string();
    wait_run(&fixture, &id, "awaiting_permission")?;
    jj(&workspace, &["new", "-m", "Switched while agent waiting"])?;
    let switched = recorded_commit(&workspace)?;
    fixture.success(&[
        "run",
        "permission",
        "--run",
        &id,
        "--permission",
        "1",
        "--option",
        "allow",
    ])?;
    let completed = wait_run(&fixture, &id, "completed")?;
    assert_eq!(
        completed["run"]["snapshot"]["status"], "failed",
        "{completed}"
    );
    assert_eq!(
        fs::read_to_string(workspace.join("approved.txt"))?,
        "approved once\n"
    );
    assert_eq!(
        recorded_commit(&workspace)?,
        switched,
        "wrong change must not be snapshotted or repaired"
    );
    Ok(())
}

#[test]
fn continuation_protocol_failures_never_create_a_fresh_session_or_send_a_prompt()
-> anyhow::Result<()> {
    let (fixture, agent, workspace) = run_fixture("complete")?;
    let first = start_run(&fixture, &agent)?;
    let mut id = first["run"]["id"].to_string();
    wait_run(&fixture, &id, "completed")?;
    for (flag, contents, expected) in [
        ("load-capabilities.json", "{}", "loadSession"),
        (
            "load-capabilities.json",
            r#"{"loadSession":false}"#,
            "loadSession",
        ),
        ("load-error", "", "no rollout found"),
        ("load-malformed", "", "must be an object"),
        ("load-wrong-session", "", "different session"),
        ("load-wrong-update", "", "wrong session"),
        ("mode-error", "", "unsupported mode"),
    ] {
        let before = agent_requests(&agent)?.len();
        fs::write(workspace.join(flag), contents)?;
        let started = continue_run(&fixture, &id, "must not be sent")?;
        id = started["run"]["id"].to_string();
        let failed = wait_run(&fixture, &id, "failed")?;
        assert!(
            failed["run"]["state"]["detail"]
                .as_str()
                .expect("failure")
                .contains(expected),
            "{failed}"
        );
        assert_process_stopped(&workspace.join("adapter.pid"))?;
        let requests = agent_requests(&agent)?;
        for request in &requests[before..] {
            assert_ne!(request["method"], "session/new", "{requests:?}");
            assert_ne!(request["method"], "session/prompt", "{requests:?}");
        }
        fs::remove_file(workspace.join(flag))?;
        fixture.success(&["backend", "status"])?;
    }
    // Explicit retry after fixing the cause uses the same session, not a fallback.
    let retry = continue_run(&fixture, &id, "complete")?;
    wait_run(&fixture, &retry["run"]["id"].to_string(), "completed")?;
    Ok(())
}

#[test]
fn continuation_reuses_permissions_cancellation_and_bounded_replay() -> anyhow::Result<()> {
    let (fixture, agent, workspace) = run_fixture("complete")?;
    let first = start_run(&fixture, &agent)?;
    let mut id = first["run"]["id"].to_string();
    wait_run(&fixture, &id, "completed")?;
    fs::write(workspace.join("load-flood"), "")?;
    for option in ["deny", "allow"] {
        let started = continue_run(&fixture, &id, "permission")?;
        id = started["run"]["id"].to_string();
        wait_run(&fixture, &id, "awaiting_permission")?;
        let events = fixture.success(&["run", "events", "--run", &id])?;
        assert_eq!(events["gap"], true);
        assert!(events["events"].as_array().expect("events").len() <= 128);
        assert_eq!(events["events"][0]["origin"], "replay");
        assert_eq!(
            events["events"]
                .as_array()
                .expect("events")
                .last()
                .expect("last")["origin"],
            "live"
        );
        assert_eq!(events["status"]["permission"]["id"], 1);
        fixture.success(&["backend", "status"])?;
        let before = agent_requests(&agent)?.len();
        let overlap = fixture
            .command()
            .args(["run", "continue", "--run", &id, "--prompt", "complete"])
            .output()?;
        assert!(!overlap.status.success());
        assert_eq!(agent_requests(&agent)?.len(), before);
        fixture.success(&[
            "run",
            "permission",
            "--run",
            &id,
            "--permission",
            "1",
            "--option",
            option,
        ])?;
        wait_run(
            &fixture,
            &id,
            if option == "allow" {
                "completed"
            } else {
                "cancelled"
            },
        )?;
        assert_eq!(workspace.join("approved.txt").exists(), option == "allow");
    }
    fs::remove_file(workspace.join("load-flood"))?;
    let started = continue_run(&fixture, &id, "hang")?;
    id = started["run"]["id"].to_string();
    wait_run(&fixture, &id, "running")?;
    let deadline = Instant::now() + Duration::from_secs(3);
    while !workspace.join("descendant.pid").exists() {
        assert!(Instant::now() < deadline, "descendant never started");
        std::thread::sleep(Duration::from_millis(10));
    }
    fixture.success(&["run", "cancel", "--run", &id])?;
    wait_run(&fixture, &id, "cancelled")?;
    assert_process_stopped(&workspace.join("adapter.pid"))?;
    assert_process_stopped(&workspace.join("descendant.pid"))?;
    // Loading itself is cancellable, before any follow-up is sent.
    fs::write(workspace.join("stall-load"), "")?;
    let started = continue_run(&fixture, &id, "must not be sent")?;
    id = started["run"]["id"].to_string();
    let deadline = Instant::now() + Duration::from_secs(3);
    while fixture.success(&["run", "events", "--run", &id])?["next"] == 0 {
        assert!(Instant::now() < deadline, "load never replayed");
        std::thread::sleep(Duration::from_millis(10));
    }
    fixture.success(&["run", "cancel", "--run", &id])?;
    wait_run(&fixture, &id, "cancelled")?;
    assert_process_stopped(&workspace.join("adapter.pid"))?;
    assert_eq!(
        fs::read_to_string(workspace.join("captured-prompt.txt"))?,
        "hang"
    );
    Ok(())
}

#[test]
fn continuation_preflight_rejects_invalid_sources_prompts_and_native_changes_before_launch()
-> anyhow::Result<()> {
    let (fixture, agent, workspace) = run_fixture("complete")?;
    let first = start_run(&fixture, &agent)?;
    let id = first["run"]["id"].to_string();
    wait_run(&fixture, &id, "completed")?;
    let before = agent_requests(&agent)?.len();
    for params in [
        json!({"run_id":1}),
        json!({"run_id":1,"prompt":"complete","executable":agent}),
        json!({"run_id":1,"prompt":false}),
    ] {
        let response = fixture.rpc(
            format!(
                "{}\n",
                json!({"jsonrpc":"2.0","id":1,"method":"run.continue","params":params})
            )
            .as_bytes(),
        )?;
        assert_eq!(response["error"]["code"], -32602);
    }
    for params in [
        json!({"run_id":999,"prompt":"complete"}),
        json!({"run_id":1,"prompt":" \n\t"}),
        json!({"run_id":1,"prompt":"x".repeat(128 * 1024 + 1)}),
    ] {
        let response = fixture.rpc(
            format!(
                "{}\n",
                json!({"jsonrpc":"2.0","id":1,"method":"run.continue","params":params})
            )
            .as_bytes(),
        )?;
        assert_eq!(response["error"]["code"], -32000);
    }
    assert_eq!(agent_requests(&agent)?.len(), before);
    // A stale source cannot pretend to fork an already advanced harness session.
    let second = continue_run(&fixture, &id, "complete")?;
    let second_id = second["run"]["id"].to_string();
    wait_run(&fixture, &second_id, "completed")?;
    let relocated_agent = agent.with_extension("relocated");
    fs::rename(&agent, &relocated_agent)?;
    std::os::unix::fs::symlink(&relocated_agent, &agent)?;
    let redirected = fixture
        .command()
        .args([
            "run", "continue", "--run", &second_id, "--prompt", "complete",
        ])
        .output()?;
    assert!(!redirected.status.success());
    let redirected_stderr = String::from_utf8(redirected.stderr)?;
    assert!(
        redirected_stderr
            .contains("Saved ACP executable is missing or resolves to a different path"),
        "stderr: {redirected_stderr}"
    );
    fs::remove_file(&agent)?;
    fs::rename(&relocated_agent, &agent)?;
    let old = fixture
        .command()
        .args(["run", "continue", "--run", &id, "--prompt", "complete"])
        .output()?;
    assert!(!old.status.success());
    assert!(String::from_utf8(old.stderr)?.contains("newer run"));
    jj(&workspace, &["new", "-m", "Externally switched"])?;
    let before = agent_requests(&agent)?.len();
    let switched = fixture
        .command()
        .args([
            "run", "continue", "--run", &second_id, "--prompt", "complete",
        ])
        .output()?;
    assert!(!switched.status.success());
    assert_eq!(agent_requests(&agent)?.len(), before);
    assert_eq!(
        fixture
            .success(&["run", "list", "--workflow", "1", "--node", "1"])?
            .as_array()
            .expect("runs")
            .len(),
        2
    );
    Ok(())
}

#[test]
fn runs_remain_controllable_and_permissions_are_explicit_single_use() -> anyhow::Result<()> {
    let (fixture, agent, workspace) = run_fixture("permission")?;
    let started = start_run(&fixture, &agent)?;
    assert_eq!(started["run"]["state"]["status"], "starting");
    let id = started["run"]["id"].to_string();
    let status = wait_run(&fixture, &id, "awaiting_permission")?;
    assert_eq!(status["run"]["session_id"], "test-session");
    assert_eq!(
        status["permission"]["options"]
            .as_array()
            .expect("options")
            .len(),
        2
    );
    assert!(
        !workspace.join("approved.txt").exists(),
        "no implicit approval"
    );
    assert!(fixture.success(&["backend", "status"])?["pid"].is_number());
    assert_eq!(
        fixture.success(&["workflow", "graph", "--workflow", "1"])?["nodes"]
            .as_array()
            .expect("nodes")
            .len(),
        1
    );
    assert!(
        !fixture
            .command()
            .args([
                "run",
                "start",
                "--workflow",
                "1",
                "--node",
                "1",
                "--executable",
                agent.to_str().expect("agent"),
            ])
            .output()?
            .status
            .success(),
        "overlapping run must be refused"
    );
    let listing = fixture.success(&["run", "list", "--workflow", "1", "--node", "1"])?;
    assert_eq!(listing.as_array().expect("runs").len(), 1);
    let page = fixture.success(&["run", "events", "--run", &id])?;
    assert_eq!(page["events"][0]["update"]["content"]["text"], "event-0");
    let cursor = page["next"].to_string();
    assert_eq!(
        fixture.success(&["run", "events", "--run", &id, "--after", &cursor])?["events"],
        json!([])
    );
    for (permission, option) in [("999", "allow"), ("1", "blanket"), ("1", "unknown")] {
        assert!(
            !fixture
                .command()
                .args([
                    "run",
                    "permission",
                    "--run",
                    &id,
                    "--permission",
                    permission,
                    "--option",
                    option,
                ])
                .output()?
                .status
                .success()
        );
    }
    fixture.success(&[
        "node",
        "update",
        "--workflow",
        "1",
        "--node",
        "1",
        "--prompt",
        "new prompt",
    ])?;
    assert_eq!(
        fs::read_to_string(workspace.join("captured-prompt.txt"))?,
        "permission"
    );
    assert_eq!(
        status["run"]["prompt"], "permission",
        "prompt captured at start"
    );
    fixture.success(&[
        "run",
        "permission",
        "--run",
        &id,
        "--permission",
        "1",
        "--option",
        "deny",
    ])?;
    wait_run(&fixture, &id, "cancelled")?;
    assert!(!workspace.join("approved.txt").exists());
    assert_process_stopped(&workspace.join("adapter.pid"))?;
    assert!(
        !fixture
            .command()
            .args([
                "run",
                "permission",
                "--run",
                &id,
                "--permission",
                "1",
                "--option",
                "allow",
            ])
            .output()?
            .status
            .success(),
        "duplicate reply must fail"
    );
    fixture.success(&[
        "node",
        "update",
        "--workflow",
        "1",
        "--node",
        "1",
        "--prompt",
        "permission",
    ])?;
    let second = start_run(&fixture, &agent)?;
    let second = second["run"]["id"].to_string();
    wait_run(&fixture, &second, "awaiting_permission")?;
    fixture.success(&[
        "run",
        "permission",
        "--run",
        &second,
        "--permission",
        "1",
        "--option",
        "allow",
    ])?;
    wait_run(&fixture, &second, "completed")?;
    assert_eq!(
        fs::read_to_string(workspace.join("approved.txt"))?,
        "approved once\n"
    );
    fixture.stop()?;
    let saved = fixture.success(&["run", "status", "--run", &second])?;
    assert_eq!(saved["run"]["state"]["status"], "completed");
    assert!(saved["permission"].is_null());
    assert!(
        !fixture
            .command()
            .args(["run", "events", "--run", &second])
            .output()?
            .status
            .success(),
        "live output must not masquerade as durable history"
    );
    Ok(())
}

#[test]
fn cancellation_and_shutdown_kill_the_owned_process_group() -> anyhow::Result<()> {
    let (fixture, agent, workspace) = run_fixture("hang")?;
    for stop_backend in [false, true] {
        let started = start_run(&fixture, &agent)?;
        let id = started["run"]["id"].to_string();
        wait_run(&fixture, &id, "running")?;
        let deadline = Instant::now() + Duration::from_secs(3);
        while !workspace.join("descendant.pid").exists() {
            assert!(Instant::now() < deadline, "descendant never started");
            std::thread::sleep(Duration::from_millis(10));
        }
        if stop_backend {
            fixture.stop()?;
        } else {
            let cancelling = fixture.success(&["run", "cancel", "--run", &id])?;
            assert_eq!(cancelling["run"]["state"]["status"], "cancelling");
            wait_run(&fixture, &id, "cancelled")?;
        }
        assert_process_stopped(&workspace.join("adapter.pid"))?;
        assert_process_stopped(&workspace.join("descendant.pid"))?;
        assert_eq!(
            fixture.success(&["run", "status", "--run", &id])?["run"]["state"]["status"],
            "cancelled"
        );
        fs::remove_file(workspace.join("descendant.pid"))?;
    }
    Ok(())
}

#[test]
fn unanswered_permission_expires_without_granting_access() -> anyhow::Result<()> {
    let (fixture, agent, workspace) = run_fixture("permission")?;
    let started = start_run(&fixture, &agent)?;
    let id = started["run"]["id"].to_string();
    wait_run(&fixture, &id, "awaiting_permission")?;
    fixture.success(&["backend", "status"])?;
    wait_run_for(&fixture, &id, "cancelled", Duration::from_secs(70))?;
    assert!(!workspace.join("approved.txt").exists());
    assert_process_stopped(&workspace.join("adapter.pid"))?;
    // Reservation released only after confirmed process cleanup.
    let next = start_run(&fixture, &agent)?;
    let next = next["run"]["id"].to_string();
    wait_run(&fixture, &next, "awaiting_permission")?;
    fixture.success(&["run", "cancel", "--run", &next])?;
    wait_run(&fixture, &next, "cancelled")?;
    Ok(())
}

#[test]
fn backend_crash_retains_uncertain_run_reservation_without_replaying_prompt() -> anyhow::Result<()>
{
    let (fixture, agent, workspace) = run_fixture("permission")?;
    let started = start_run(&fixture, &agent)?;
    let id = started["run"]["id"].to_string();
    let waiting = wait_run(&fixture, &id, "awaiting_permission")?;
    assert!(waiting["run"]["process_id"].is_number());
    let backend_pid = fixture.success(&["backend", "status"])?["pid"].to_string();
    let killed = Command::new("/bin/kill")
        .args(["-KILL", &backend_pid])
        .status()?;
    assert!(killed.success(), "failed to kill owned test backend");
    // Test cleanup, not application recovery: stop only this fixture's known group.
    let agent_pid = fs::read_to_string(workspace.join("adapter.pid"))?;
    let _ = Command::new("/bin/kill")
        .args(["-KILL", "--", &format!("-{agent_pid}")])
        .status()?;
    assert_process_stopped(&workspace.join("adapter.pid"))?;
    let restarted = fixture.success(&["run", "status", "--run", &id])?;
    assert_eq!(restarted["run"]["state"]["status"], "interrupted");
    assert_eq!(restarted["run"]["session_id"], waiting["run"]["session_id"]);
    assert_eq!(restarted["run"]["process_id"], waiting["run"]["process_id"]);
    assert!(restarted["permission"].is_null());
    assert!(!workspace.join("approved.txt").exists());
    let continuation = fixture
        .command()
        .args(["run", "continue", "--run", &id, "--prompt", "complete"])
        .output()?;
    assert!(
        !continuation.status.success(),
        "interrupted runs must not resume editing"
    );
    assert!(String::from_utf8(continuation.stderr)?.contains("confirmed cleanup"));
    assert!(
        !fixture
            .command()
            .args([
                "run",
                "start",
                "--workflow",
                "1",
                "--node",
                "1",
                "--executable",
                agent.to_str().expect("agent"),
            ])
            .output()?
            .status
            .success(),
        "restart must not free an uncertain reservation"
    );
    assert_eq!(
        fixture
            .success(&["run", "list", "--workflow", "1", "--node", "1"])?
            .as_array()
            .expect("runs")
            .len(),
        1,
        "no prompt automatically replayed"
    );
    Ok(())
}

#[test]
fn bounded_events_report_gaps_and_bad_agents_fail_without_blocking_backend() -> anyhow::Result<()> {
    let (fixture, agent, workspace) = run_fixture("flood")?;
    let started = start_run(&fixture, &agent)?;
    let id = started["run"]["id"].to_string();
    wait_run(&fixture, &id, "awaiting_permission")?;
    let events = fixture.success(&["run", "events", "--run", &id])?;
    assert_eq!(events["gap"], true);
    assert!(events["events"].as_array().expect("events").len() <= 128);
    assert_eq!(events["next"], 300);
    assert_eq!(events["status"]["permission"]["id"], 1);
    fixture.success(&["run", "cancel", "--run", &id])?;
    wait_run(&fixture, &id, "cancelled")?;
    for prompt in ["malformed", "exit", "oversize", "complete"] {
        fixture.success(&[
            "node",
            "update",
            "--workflow",
            "1",
            "--node",
            "1",
            "--prompt",
            prompt,
        ])?;
        let before = recorded_commit(&workspace)?;
        let started = start_run(&fixture, &agent)?;
        let id = started["run"]["id"].to_string();
        let stopped = wait_run(
            &fixture,
            &id,
            if prompt == "complete" {
                "completed"
            } else {
                "failed"
            },
        )?;
        if prompt != "complete" {
            assert_eq!(stopped["run"]["snapshot"]["status"], "not_recorded");
            assert_eq!(
                recorded_commit(&workspace)?,
                before,
                "failed agent must not snapshot"
            );
        }
        fixture.success(&["backend", "status"])?;
    }
    let invalid = fixture.rpc(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"run.cancel\",\"params\":{\"run_id\":1,\"extra\":true}}\n")?;
    assert_eq!(invalid["error"]["code"], -32602);
    Ok(())
}

#[test]
fn run_start_validates_before_launch_and_setup_can_be_cancelled() -> anyhow::Result<()> {
    let (fixture, agent, workspace) = run_fixture("complete")?;
    let wrong_workflow = fixture
        .command()
        .args([
            "run",
            "start",
            "--workflow",
            "999",
            "--node",
            "1",
            "--executable",
            agent.to_str().expect("agent"),
        ])
        .output()?;
    assert!(!wrong_workflow.status.success());
    let invalid_path = fixture.rpc(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"run.start\",\"params\":{\"workflow_id\":1,\"node_id\":1,\"executable\":\"relative\"}}\n")?;
    assert_eq!(invalid_path["error"]["code"], -32000);
    let long_prompt = "x".repeat(128 * 1024 + 1);
    let mut request = serde_json::to_vec(&json!({
        "jsonrpc":"2.0", "id":1, "method":"node.update",
        "params":{"workflow_id":1,"node_id":1,"prompt":long_prompt}
    }))?;
    request.push(b'\n');
    assert!(fixture.rpc(&request)?.get("result").is_some());
    let oversized = fixture
        .command()
        .args([
            "run",
            "start",
            "--workflow",
            "1",
            "--node",
            "1",
            "--executable",
            agent.to_str().expect("agent"),
        ])
        .output()?;
    assert!(!oversized.status.success());
    assert!(String::from_utf8(oversized.stderr)?.contains("128 KiB"));
    assert_eq!(
        fixture.success(&["run", "list", "--workflow", "1", "--node", "1"])?,
        json!([])
    );
    assert!(
        !workspace.join("adapter.pid").exists(),
        "invalid starts never launch"
    );
    fixture.success(&[
        "node",
        "update",
        "--workflow",
        "1",
        "--node",
        "1",
        "--prompt",
        "complete",
    ])?;
    fs::write(workspace.join("stall-initialize"), "")?;
    let started = start_run(&fixture, &agent)?;
    let id = started["run"]["id"].to_string();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !workspace.join("adapter.pid").exists() {
        assert!(Instant::now() < deadline, "adapter never initialized");
        std::thread::sleep(Duration::from_millis(10));
    }
    fixture.success(&["backend", "status"])?;
    fixture.success(&["run", "cancel", "--run", &id])?;
    wait_run(&fixture, &id, "cancelled")?;
    assert_process_stopped(&workspace.join("adapter.pid"))?;
    fs::remove_file(workspace.join("stall-initialize"))?;
    let before = agent_requests(&agent)?.len();
    let continuation = fixture
        .command()
        .args(["run", "continue", "--run", &id, "--prompt", "complete"])
        .output()?;
    assert!(!continuation.status.success());
    let continuation_stderr = String::from_utf8(continuation.stderr)?;
    assert!(
        continuation_stderr
            .to_ascii_lowercase()
            .contains("no saved acp session"),
        "stderr: {continuation_stderr}"
    );
    assert_eq!(agent_requests(&agent)?.len(), before);
    let next = start_run(&fixture, &agent)?;
    wait_run(&fixture, &next["run"]["id"].to_string(), "completed")?;
    jj(&workspace, &["new", "-m", "Switched externally"])?;
    let switched = fixture
        .command()
        .args([
            "run",
            "start",
            "--workflow",
            "1",
            "--node",
            "1",
            "--executable",
            agent.to_str().expect("agent"),
        ])
        .output()?;
    assert!(
        !switched.status.success(),
        "switched workspace must be refused"
    );
    Ok(())
}

#[test]
fn cli_coordinator_opens_the_exact_workflow_while_a_fake_worker_is_active() -> anyhow::Result<()> {
    let (fixture, agent, _workspace) = run_fixture("permission")?;
    let project = fixture.directory.path().join("project");
    let project = fs::canonicalize(project)?;

    let started = start_run(&fixture, &agent)?;
    let run_id = started["run"]["id"].to_string();
    let status = wait_run(&fixture, &run_id, "awaiting_permission")?;
    assert!(
        status["permission"].is_object(),
        "the fake ACP worker reached a permission request: {status}"
    );

    let desktop = tempfile::Builder::new()
        .prefix("wp-desktop-")
        .tempdir_in("/tmp")?;
    let socket = desktop.path().join("desktop.sock");
    let listener = UnixListener::bind(&socket)?;
    let expected_project = project.clone();
    let listener_thread = std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept app open");
        let mut reader = BufReader::new(stream.try_clone().expect("clone"));
        let request: Request = waypoints_desktop_ipc::read_request(&mut reader)
            .expect("read app open request")
            .expect("request");
        assert_eq!(request.project.as_ref(), Some(&expected_project));
        assert_eq!(request.workflow, Some(1));
        let mut writer = stream;
        waypoints_desktop_ipc::write_response(&mut writer, &Response::focused(&request))
            .expect("write app open response");
    });

    let opened = fixture
        .command()
        .env("WAYPOINTS_APP_SOCKET", &socket)
        .args([
            "app",
            "open",
            "--project",
            project.to_str().expect("project path"),
            "--workflow",
            "1",
        ])
        .output()?;
    let opened = fixture.json_output(opened, "app open")?;
    listener_thread.join().expect("listener thread");
    assert_eq!(opened["status"], "focused");
    assert_eq!(opened["workflow"], 1);

    // Opening the visual is not creation: the coordinator still has one workflow
    // and the external worker run remains the only run.
    let workflows = fixture.success(&[
        "workflow",
        "list",
        "--project",
        project.to_str().expect("project path"),
    ])?;
    assert_eq!(workflows.as_array().map(Vec::len), Some(1));
    let runs = fixture.success(&["run", "list", "--workflow", "1", "--node", "1"])?;
    assert_eq!(runs.as_array().map(Vec::len), Some(1));

    fixture.success(&["run", "cancel", "--run", &run_id])?;
    wait_run(&fixture, &run_id, "cancelled")?;
    Ok(())
}

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
fn cli_allocates_node_workspace_at_client_relative_path_and_reuses_it() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    jj(fixture.directory.path(), &["git", "init", "project"])?;
    let project = fixture.directory.path().join("project");
    fs::write(project.join("interfaces.txt"), "base interface\n")?;
    jj(&project, &["describe", "-m", "Base"])?;
    let original = jj(
        &project,
        &["log", "--no-graph", "-r", "@", "-T", "commit_id"],
    )?;
    let workflow = fixture.success(&[
        "workflow",
        "create",
        "--project",
        project.to_str().expect("project path"),
        "--name",
        "Workspace",
    ])?["id"]
        .to_string();
    let node = fixture.success(&[
        "node",
        "create",
        "--workflow",
        &workflow,
        "--prompt",
        "Authentication",
    ])?;
    let node_id = node["id"].to_string();
    assert_eq!(
        fixture.success(&[
            "node",
            "workspace",
            "status",
            "--workflow",
            &workflow,
            "--node",
            &node_id,
        ])?["state"]["status"],
        "unallocated"
    );
    let change = fixture.success(&[
        "node",
        "create-change",
        "--workflow",
        &workflow,
        "--node",
        &node_id,
        "--base",
        &original,
    ])?;
    fs::write(project.join("dirty.txt"), "keep dirty\n")?;
    // The already-running backend has a different cwd. The CLI must resolve this.
    let args = [
        "node",
        "workspace",
        "create",
        "--workflow",
        &workflow,
        "--node",
        &node_id,
        "--directory",
        "../authentication",
    ];
    let output = fixture
        .command()
        .current_dir(&project)
        .args(args)
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let created: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(created["node"], node);
    assert_eq!(created["state"]["status"], "ready");
    assert_eq!(
        created["workspace"]["change_id"],
        change["association"]["change_id"]
    );
    let path = fixture
        .directory
        .path()
        .join("authentication")
        .canonicalize()?;
    assert_eq!(
        created["workspace"]["directory"],
        path.to_str().expect("workspace path")
    );
    assert_eq!(
        fs::read_to_string(path.join("interfaces.txt"))?,
        "base interface\n"
    );
    assert_eq!(
        fs::read_to_string(project.join("dirty.txt"))?,
        "keep dirty\n"
    );
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
        original
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
    let repeat = fixture
        .command()
        .current_dir(&project)
        .args(args)
        .output()?;
    assert!(repeat.status.success());
    assert_eq!(serde_json::from_slice::<Value>(&repeat.stdout)?, created);
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
    fs::write(path.join("auth.txt"), "implementation\n")?;
    jj(&path, &["status"])?;
    let refreshed = fixture.success(&[
        "node",
        "refresh",
        "--workflow",
        &workflow,
        "--node",
        &node_id,
    ])?;
    assert_eq!(refreshed["association"], change["association"]);
    assert_ne!(
        refreshed["state"]["revision"]["commit_id"],
        change["state"]["revision"]["commit_id"]
    );
    let status = fixture.success(&[
        "node",
        "workspace",
        "status",
        "--workflow",
        &workflow,
        "--node",
        &node_id,
    ])?;
    assert_eq!(status["state"]["revision"], refreshed["state"]["revision"]);
    for (method, params) in [
        (
            "node.workspace.create",
            json!({"workflow_id": 1, "node_id": 1}),
        ),
        (
            "node.workspace.create",
            json!({"workflow_id": 1, "node_id": 1, "directory": "/tmp/not-created", "extra": true}),
        ),
        (
            "node.workspace.status",
            json!({"workflow_id": 1, "node_id": 1, "extra": true}),
        ),
        (
            "node.workspace.verify",
            json!({"workflow_id": 1, "node_id": 1, "extra": true}),
        ),
        ("node.workspace.verify", json!({"workflow_id": 1})),
    ] {
        let response = fixture.rpc(
            format!(
                "{}\n",
                json!({
                    "jsonrpc":"2.0", "id":1, "method":method, "params":params,
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
            "workspace",
            "status",
            "--workflow",
            &workflow,
            "--node",
            &node_id
        ])?,
        status
    );
    let human = Command::new(env!("CARGO_BIN_EXE_waypoints"))
        .arg("--database")
        .arg(&fixture.database)
        .arg("--socket")
        .arg(&fixture.socket)
        .args([
            "node",
            "workspace",
            "verify",
            "--workflow",
            &workflow,
            "--node",
            &node_id,
        ])
        .output()?;
    assert!(human.status.success());
    let stdout = String::from_utf8(human.stdout)?;
    assert!(stdout.contains("Workspace status: ready"));
    assert!(stdout.contains(path.to_str().expect("workspace path")));
    fixture.stop()?;
    Ok(())
}

#[test]
fn interrupted_workspace_allocation_remains_inspectable_and_never_retries() -> anyhow::Result<()> {
    let native = Command::new("/bin/sh")
        .args(["-c", "command -v jj"])
        .output()?;
    assert!(native.status.success());
    let native = String::from_utf8(native.stdout)?.trim().to_owned();
    for fail_at in ["add", "checkout", "completed"] {
        let fixture = Fixture::new()?;
        jj(fixture.directory.path(), &["git", "init", "project"])?;
        let project = fixture.directory.path().join("project");
        fs::write(project.join("base.txt"), "base contents\n")?;
        jj(&project, &["describe", "-m", "Original"])?;
        let base = jj(
            &project,
            &["log", "--no-graph", "-r", "@", "-T", "commit_id"],
        )?;
        let bin = fixture.directory.path().join("bin");
        fs::create_dir(&bin)?;
        let wrapper = bin.join("jj");
        let injected = if fail_at == "add" {
            "case \" $* \" in *' workspace add '*) \"$NATIVE_JJ\" \"$@\"; echo 'injected add failure' >&2; exit 1;; esac"
        } else if fail_at == "checkout" {
            // Exceed the ordinary five-second RPC timeout to verify the workspace deadline.
            "case \" $* \" in *' workspace update-stale '*) /bin/sleep 6; echo 'injected checkout failure' >&2; exit 1;; esac"
        } else {
            // Checkout succeeds, but the backend never receives a success outcome.
            "case \" $* \" in *' workspace update-stale '*) \"$NATIVE_JJ\" \"$@\"; echo 'injected completion failure' >&2; exit 1;; esac"
        };
        fs::write(
            &wrapper,
            format!("#!/bin/sh\n{injected}\nexec \"$NATIVE_JJ\" \"$@\"\n"),
        )?;
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700))?;
        assert!(
            fixture
                .list_command()
                .env("PATH", &bin)
                .env("NATIVE_JJ", &native)
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
            "Interrupted",
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
        fixture.success(&[
            "node",
            "create-change",
            "--workflow",
            &workflow,
            "--node",
            &node,
            "--base",
            &base,
        ])?;
        let destination = fixture.directory.path().join("workspace");
        let args = [
            "node",
            "workspace",
            "create",
            "--workflow",
            &workflow,
            "--node",
            &node,
            "--directory",
            destination.to_str().expect("destination path"),
        ];
        let failure = fixture.command().args(args).output()?;
        assert!(!failure.status.success());
        let error = String::from_utf8(failure.stderr)?;
        assert!(error.contains("workspace creation incomplete"), "{error}");
        assert!(
            error.contains("injected"),
            "workspace RPC must wait for native outcome: {error}"
        );
        assert!(
            destination.join(".jj").exists(),
            "native artifacts must be preserved"
        );
        fs::write(destination.join("keep.txt"), "do not delete or overwrite\n")?;
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
        let pending = fixture.success(&[
            "node",
            "workspace",
            "status",
            "--workflow",
            &workflow,
            "--node",
            &node,
        ])?;
        assert_eq!(pending["state"]["status"], "incomplete");
        assert!(
            pending["workspace"]["name"]
                .as_str()
                .expect("saved name")
                .starts_with("waypoints-")
        );
        fixture.stop()?;
        assert_eq!(
            fixture.success(&args)?,
            pending,
            "repeat reports allocation without retrying native work"
        );
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
        assert_eq!(
            fs::read_to_string(destination.join("keep.txt"))?,
            "do not delete or overwrite\n"
        );
        assert!(
            fixture
                .command()
                .args([
                    "node",
                    "workspace",
                    "create",
                    "--workflow",
                    &workflow,
                    "--node",
                    &node,
                    "--directory",
                    fixture
                        .directory
                        .path()
                        .join("replacement")
                        .to_str()
                        .expect("replacement path"),
                ])
                .output()?
                .status
                .code()
                .is_some_and(|code| code != 0)
        );
        let verify_args = [
            "node",
            "workspace",
            "verify",
            "--workflow",
            &workflow,
            "--node",
            &node,
        ];
        let checkout = fs::read(destination.join(".jj/working_copy/checkout"))?;
        let tree_state = fs::read(destination.join(".jj/working_copy/tree_state"))?;
        let verification = fixture.command().args(verify_args).output()?;
        if fail_at == "completed" {
            assert!(
                verification.status.success(),
                "{}",
                String::from_utf8_lossy(&verification.stderr)
            );
            let verified: Value = serde_json::from_slice(&verification.stdout)?;
            assert_eq!(verified["state"]["status"], "ready");
            assert_eq!(verified["workspace"], pending["workspace"]);
            assert_eq!(
                fs::read_to_string(destination.join("base.txt"))?,
                "base contents\n"
            );
            assert_eq!(
                fixture.success(&verify_args)?,
                verified,
                "verification is idempotent"
            );
            fixture.stop()?;
            assert_eq!(
                fixture.success(&[
                    "node",
                    "workspace",
                    "status",
                    "--workflow",
                    &workflow,
                    "--node",
                    &node,
                ])?,
                verified
            );
        } else {
            assert!(
                !verification.status.success(),
                "must reject incomplete {fail_at}"
            );
            let error = String::from_utf8_lossy(&verification.stderr);
            assert!(
                error.contains("inspect") || error.contains("checkout"),
                "{error}"
            );
            assert_eq!(
                fixture.success(&[
                    "node",
                    "workspace",
                    "status",
                    "--workflow",
                    &workflow,
                    "--node",
                    &node,
                ])?,
                pending,
                "failed verification must leave readiness unchanged"
            );
        }
        assert_eq!(
            fs::read(destination.join(".jj/working_copy/checkout"))?,
            checkout
        );
        assert_eq!(
            fs::read(destination.join(".jj/working_copy/tree_state"))?,
            tree_state
        );
        assert_eq!(
            fs::read_to_string(destination.join("keep.txt"))?,
            "do not delete or overwrite\n"
        );
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
                    "id",
                ]
            )?,
            operation,
            "verification must not mutate jj history"
        );
        fixture.stop()?;
    }
    Ok(())
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
    for (params, code) in [
        (json!({"workflow_id": 1, "node_id": 1}), -32000),
        (
            json!({"workflow_id": 1, "node_id": 1, "base": "root()", "unexpected": true}),
            -32602,
        ),
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
        assert_eq!(response["error"]["code"], code);
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
            "base_nodes": [],
            "connectivity": {"status": "unbased"},
        })
    );
    let other_graph = fixture.success(&["workflow", "graph", "--workflow", &other])?;
    assert_eq!(
        other_graph,
        json!({"workflow": second, "nodes": [other_node], "connections": [], "base_nodes": [], "connectivity": {"status": "unbased"}})
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
fn rooted_node_cli_and_rpc_require_dependencies_and_create_native_work_from_them()
-> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    jj(fixture.directory.path(), &["git", "init", "project"])?;
    let project_path = fixture.directory.path().join("project");
    let project = project_path.to_str().expect("project path");
    jj(&project_path, &["describe", "-m", "Base"])?;
    jj(&project_path, &["bookmark", "create", "main"])?;
    fixture.success(&[
        "workflow",
        "create",
        "--project",
        project,
        "--name",
        "Rooted",
        "--base",
        "main",
    ])?;
    let missing = fixture
        .command()
        .args(["node", "create", "--workflow", "1", "--prompt", "Orphan"])
        .output()?;
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("--from-base or --parent"));
    fixture.success(&[
        "node",
        "create",
        "--workflow",
        "1",
        "--prompt",
        "Interfaces",
        "--from-base",
    ])?;
    fixture.success(&[
        "node",
        "create",
        "--workflow",
        "1",
        "--prompt",
        "Authentication",
        "--parent",
        "1",
    ])?;
    let graph = fixture.success(&["workflow", "graph", "--workflow", "1"])?;
    assert_eq!(graph["connectivity"]["status"], "rooted");
    assert_eq!(graph["base_nodes"], json!([1]));
    assert_eq!(
        graph["connections"],
        json!([{"workflow_id":1,"source_id":1,"target_id":2}])
    );
    let history = fixture.success(&["history", "status", "--project", project])?;
    let edit = history["undo"]["id"].to_string();
    fixture.success(&["history", "undo", "--project", project, "--expected", &edit])?;
    assert_eq!(
        fixture.success(&["workflow", "graph", "--workflow", "1"])?["connections"],
        json!([])
    );
    fixture.stop()?;
    fixture.success(&["history", "redo", "--project", project, "--expected", &edit])?;
    assert_eq!(
        fixture.success(&["workflow", "graph", "--workflow", "1"])?,
        graph
    );
    for parent in [
        json!({"kind":"node","node_id":999}),
        json!({"kind":"workflow_base","force":true}),
        json!({"kind":"node"}),
    ] {
        let mut frame = serde_json::to_vec(&json!({
            "jsonrpc":"2.0","id":1,"method":"node.create",
            "params":{"workflow_id":1,"prompt":"Invalid","parent":parent}
        }))?;
        frame.push(b'\n');
        assert!(fixture.rpc(&frame)?.get("error").is_some());
    }
    let both = fixture
        .command()
        .args([
            "node",
            "create",
            "--workflow",
            "1",
            "--prompt",
            "Both",
            "--from-base",
            "--parent",
            "1",
        ])
        .output()?;
    assert!(!both.status.success());
    assert_eq!(
        fixture.success(&["workflow", "graph", "--workflow", "1"])?,
        graph
    );
    fixture.success(&["node", "create-change", "--workflow", "1", "--node", "1"])?;
    fixture.success(&["node", "create-change", "--workflow", "1", "--node", "2"])?;
    let connection = fixture
        .command()
        .args([
            "connection",
            "create",
            "--workflow",
            "1",
            "--source",
            "1",
            "--target",
            "2",
        ])
        .output()?;
    assert!(!connection.status.success());
    assert!(String::from_utf8_lossy(&connection.stderr).contains("metadata-only connection edits"));
    Ok(())
}

#[test]
fn reparent_cli_requires_acknowledgement_and_persists_native_outcomes() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    jj(fixture.directory.path(), &["git", "init", "project"])?;
    let project_path = fixture.directory.path().join("project");
    let project = project_path.to_str().expect("project path");
    fixture.success(&[
        "workflow",
        "create",
        "--project",
        project,
        "--name",
        "Reparent",
        "--base-revision",
        "root()",
    ])?;
    for (name, dependency) in [
        ("Interfaces", vec!["--from-base"]),
        ("Authentication", vec!["--parent", "1"]),
        ("Review", vec!["--parent", "2"]),
        ("Followup", vec!["--parent", "3"]),
    ] {
        let mut args = vec!["node", "create", "--workflow", "1", "--prompt", name];
        args.extend(dependency);
        fixture.success(&args)?;
    }
    for id in ["1", "2", "3", "4"] {
        fixture.success(&["node", "create-change", "--workflow", "1", "--node", id])?;
    }
    let graph = fixture.success(&["workflow", "graph", "--workflow", "1"])?;
    let history = fixture.success(&["history", "status", "--project", project])?;
    let missing = fixture
        .command()
        .args([
            "node",
            "reparent",
            "--workflow",
            "1",
            "--node",
            "3",
            "--parent",
            "1",
        ])
        .output()?;
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("--accept-undo-boundary"));
    for (extra, code) in [
        (json!({}), -32000),
        (json!({"acknowledgement":true}), -32602),
        (json!({"acknowledgement":"force"}), -32602),
        (
            json!({"acknowledgement":"accept_undo_boundary", "force":true}),
            -32602,
        ),
    ] {
        let mut params = json!({"workflow_id":1,"node_id":3,"parent":{"kind":"node","node_id":1}});
        params
            .as_object_mut()
            .expect("object")
            .extend(extra.as_object().expect("object").clone());
        let mut frame = serde_json::to_vec(
            &json!({"jsonrpc":"2.0","id":1,"method":"node.reparent","params":params}),
        )?;
        frame.push(b'\n');
        assert_eq!(fixture.rpc(&frame)?["error"]["code"], code);
    }
    assert_eq!(
        fixture.success(&["workflow", "graph", "--workflow", "1"])?,
        graph
    );
    assert_eq!(
        fixture.success(&["history", "status", "--project", project])?,
        history
    );
    let operation = fixture.success(&[
        "node",
        "reparent",
        "--workflow",
        "1",
        "--node",
        "3",
        "--parent",
        "1",
        "--accept-undo-boundary",
    ])?;
    assert_eq!(operation["state"], "completed");
    assert_eq!(operation["after"].as_array().expect("revisions").len(), 2);
    assert_eq!(
        operation["previous_parent"],
        json!({"kind":"node","node_id":2})
    );
    let graph = fixture.success(&["workflow", "graph", "--workflow", "1"])?;
    assert_eq!(
        graph["connections"],
        json!([
            {"workflow_id":1,"source_id":1,"target_id":2},
            {"workflow_id":1,"source_id":1,"target_id":3},
            {"workflow_id":1,"source_id":3,"target_id":4},
        ])
    );
    let status = fixture.success(&["history", "status", "--project", project])?;
    assert!(status["undo"].is_null() && status["redo"].is_null());
    fixture.stop()?;
    assert_eq!(
        fixture.success(&["node", "operations", "--workflow", "1"])?,
        json!([operation])
    );
    let base = fixture.success(&[
        "node",
        "reparent",
        "--workflow",
        "1",
        "--node",
        "3",
        "--from-base",
        "--accept-undo-boundary",
    ])?;
    assert_eq!(base["state"], "completed");
    assert_eq!(
        fixture.success(&["workflow", "graph", "--workflow", "1"])?["base_nodes"],
        json!([1, 3])
    );
    Ok(())
}

#[test]
fn dependency_recovery_cli_resolves_both_crash_windows_without_repeating_native_mutation()
-> anyhow::Result<()> {
    for (command, mutation, applied) in [
        ("reparent", "rebase", false),
        ("reparent", "rebase", true),
        ("delete", "abandon", false),
        ("delete", "abandon", true),
    ] {
        let fixture = Fixture::new()?;
        jj(fixture.directory.path(), &["git", "init", "project"])?;
        let project = fixture.directory.path().join("project");
        fixture.success(&[
            "workflow",
            "create",
            "--project",
            project.to_str().expect("path"),
            "--name",
            "Recovery",
            "--base-revision",
            "root()",
        ])?;
        fixture.success(&[
            "node",
            "create",
            "--workflow",
            "1",
            "--prompt",
            "A",
            "--from-base",
        ])?;
        fixture.success(&["node", "create-change", "--workflow", "1", "--node", "1"])?;
        fixture.success(&[
            "node",
            "create",
            "--workflow",
            "1",
            "--prompt",
            "B",
            "--parent",
            "1",
        ])?;
        fixture.success(&["node", "create-change", "--workflow", "1", "--node", "2"])?;
        let before_graph = fixture.success(&["workflow", "graph", "--workflow", "1"])?;
        fixture.stop()?;
        let output = Command::new("/bin/sh")
            .args(["-c", "command -v jj"])
            .output()?;
        assert!(output.status.success());
        let native = String::from_utf8(output.stdout)?.trim().to_owned();
        let bin = fixture.directory.path().join("native-bin");
        fs::create_dir(&bin)?;
        let entered = fixture.directory.path().join("rebase-entered");
        let calls = fixture.directory.path().join("rebase-calls");
        let wrapper = bin.join("jj");
        fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\nnative='{native}'\nfor arg do\n if [ \"$arg\" = {mutation} ]; then\n\
             echo {mutation} >> '{}'\n{}\necho $$ > '{}'\nexec /bin/sleep 15\n fi\ndone\nexec \"$native\" \"$@\"\n",
                calls.display(),
                if applied {
                    "\"$native\" \"$@\" > /dev/null || exit $?"
                } else {
                    ":"
                },
                entered.display(),
            ),
        )?;
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700))?;
        let path = format!("{}:{}", bin.display(), std::env::var("PATH")?);
        fixture.json_output(
            fixture.list_command().env("PATH", path).output()?,
            "start intercepted backend",
        )?;
        let backend_pid = fixture.success(&["backend", "status"])?["pid"].to_string();
        let mut args = vec![
            "node",
            command,
            "--workflow",
            "1",
            "--node",
            "2",
            "--accept-undo-boundary",
        ];
        if command == "reparent" {
            args.push("--from-base");
        }
        let child = fixture
            .command()
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        wait_for_file(&entered);
        // Only these freshly launched, fixture-owned processes are signalled.
        assert!(
            Command::new("/bin/kill")
                .args(["-KILL", &backend_pid])
                .status()?
                .success()
        );
        let wrapper_pid = fs::read_to_string(&entered)?;
        let _ = Command::new("/bin/kill")
            .args(["-KILL", wrapper_pid.trim()])
            .status()?;
        assert_process_stopped(&entered)?;
        assert!(!child.wait_with_output()?.status.success());
        fs::write(project.join("dirty-after-crash.txt"), "keep partial work\n")?;
        let native_op = jj(
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
        // Reconnect to a fresh backend with the normal native executable.
        let inspection = fixture.success(&["node", "operation-recovery", "--operation", "1"])?;
        assert_eq!(
            inspection["assessment"]["status"],
            if applied { "applied" } else { "not_applied" }
        );
        assert_eq!(inspection["operation"]["state"], "unresolved");
        assert_eq!(
            fixture.success(&["workflow", "graph", "--workflow", "1"])?["unresolved_dependency_operation"],
            1
        );
        let missing = fixture
            .command()
            .args(["node", "recover-reparent", "--operation", "1"])
            .output()?;
        assert!(!missing.status.success());
        assert!(String::from_utf8_lossy(&missing.stderr).contains("--confirm-processes-stopped"));
        for (params, code) in [
            (json!({"operation_id":1}), -32000),
            (json!({"operation_id":1,"confirmation":true}), -32602),
            (json!({"operation_id":1,"confirmation":"force"}), -32602),
            (
                json!({"operation_id":1,"confirmation":"processes_stopped","force":true}),
                -32602,
            ),
        ] {
            let mut frame = serde_json::to_vec(
                &json!({"jsonrpc":"2.0","id":1,"method":"node.recover_reparent","params":params}),
            )?;
            frame.push(b'\n');
            assert_eq!(fixture.rpc(&frame)?["error"]["code"], code);
        }
        let result = fixture.success(&[
            "node",
            "recover-reparent",
            "--operation",
            "1",
            "--confirm-processes-stopped",
        ])?;
        assert_eq!(
            result["state"],
            if applied { "completed" } else { "not_applied" }
        );
        let graph = fixture.success(&["workflow", "graph", "--workflow", "1"])?;
        assert!(graph.get("unresolved_dependency_operation").is_none());
        if applied {
            assert_eq!(
                graph["base_nodes"],
                if command == "delete" {
                    json!([1])
                } else {
                    json!([1, 2])
                }
            );
            assert_eq!(graph["connections"], json!([]));
        } else {
            assert_eq!(graph, before_graph);
        }
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
            native_op
        );
        assert_eq!(
            fs::read_to_string(&calls)?,
            format!("{mutation}\n"),
            "never repeat the native command"
        );
        assert_eq!(
            fs::read_to_string(project.join("dirty-after-crash.txt"))?,
            "keep partial work\n"
        );
        fixture.success(&[
            "node",
            "update",
            "--workflow",
            "1",
            "--node",
            "1",
            "--prompt",
            "Unblocked",
        ])?;
        fixture.stop()?;
        assert_eq!(
            fixture.success(&[
                "node",
                "recover-reparent",
                "--operation",
                "1",
                "--confirm-processes-stopped"
            ])?,
            result
        );
    }
    Ok(())
}

#[test]
fn native_node_deletion_cli_requires_consent_reconnects_children_and_persists() -> anyhow::Result<()>
{
    let fixture = Fixture::new()?;
    jj(fixture.directory.path(), &["git", "init", "project"])?;
    let project = fixture.directory.path().join("project");
    fixture.success(&[
        "workflow",
        "create",
        "--project",
        project.to_str().expect("path"),
        "--name",
        "Delete",
        "--base-revision",
        "root()",
    ])?;
    for (prompt, dependency) in [
        ("Interfaces", vec!["--from-base"]),
        ("Authentication", vec!["--parent", "1"]),
        ("Review", vec!["--parent", "2"]),
    ] {
        let mut args = vec!["node", "create", "--workflow", "1", "--prompt", prompt];
        args.extend(dependency);
        fixture.success(&args)?;
    }
    for id in ["1", "2", "3"] {
        fixture.success(&["node", "create-change", "--workflow", "1", "--node", id])?;
    }
    let graph = fixture.success(&["workflow", "graph", "--workflow", "1"])?;
    let history = fixture.success(&[
        "history",
        "status",
        "--project",
        project.to_str().expect("path"),
    ])?;
    let missing = fixture
        .command()
        .args(["node", "delete", "--workflow", "1", "--node", "2"])
        .output()?;
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("--accept-undo-boundary"));
    for (params, code) in [
        (json!({"workflow_id":1,"node_id":2}), -32000),
        (
            json!({"workflow_id":1,"node_id":2,"acknowledgement":true}),
            -32602,
        ),
        (
            json!({"workflow_id":1,"node_id":2,"acknowledgement":"accept_undo_boundary","force":true}),
            -32602,
        ),
    ] {
        let mut frame = serde_json::to_vec(
            &json!({"jsonrpc":"2.0","id":1,"method":"node.delete","params":params}),
        )?;
        frame.push(b'\n');
        assert_eq!(fixture.rpc(&frame)?["error"]["code"], code);
    }
    assert_eq!(
        fixture.success(&["workflow", "graph", "--workflow", "1"])?,
        graph
    );
    assert_eq!(
        fixture.success(&[
            "history",
            "status",
            "--project",
            project.to_str().expect("path")
        ])?,
        history
    );
    let operation = fixture.success(&[
        "node",
        "delete",
        "--workflow",
        "1",
        "--node",
        "2",
        "--accept-undo-boundary",
    ])?;
    assert_eq!(operation["kind"], "delete");
    assert_eq!(operation["state"], "completed");
    assert_eq!(operation["after"].as_array().expect("revisions").len(), 1);
    let graph = fixture.success(&["workflow", "graph", "--workflow", "1"])?;
    assert_eq!(
        graph["connections"],
        json!([{"workflow_id":1,"source_id":1,"target_id":3}])
    );
    assert_eq!(graph["nodes"].as_array().expect("nodes").len(), 2);
    assert!(
        !fixture
            .command()
            .args(["node", "get", "--workflow", "1", "--node", "2"])
            .output()?
            .status
            .success()
    );
    assert!(
        !fixture
            .command()
            .args([
                "node",
                "update",
                "--workflow",
                "1",
                "--node",
                "2",
                "--prompt",
                "No resurrection"
            ])
            .output()?
            .status
            .success()
    );
    let history = fixture.success(&[
        "history",
        "status",
        "--project",
        project.to_str().expect("path"),
    ])?;
    assert!(history["undo"].is_null() && history["redo"].is_null());
    fixture.stop()?;
    assert_eq!(
        fixture.success(&["workflow", "graph", "--workflow", "1"])?,
        graph
    );
    assert_eq!(
        fixture.success(&["node", "operations", "--workflow", "1"])?,
        json!([operation])
    );
    assert_eq!(
        fixture.success(&["node", "operation-recovery", "--operation", "1"])?["assessment"]["status"],
        "resolved"
    );
    Ok(())
}

#[test]
fn legacy_root_adoption_cli_refuses_incomplete_mapping_without_changing_the_graph()
-> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    jj(fixture.directory.path(), &["git", "init", "project"])?;
    let project_path = fixture.directory.path().join("project");
    let project = project_path.to_str().expect("project path");
    fixture.success(&[
        "workflow",
        "create",
        "--project",
        project,
        "--name",
        "Legacy",
    ])?;
    for prompt in ["First", "Second"] {
        fixture.success(&["node", "create", "--workflow", "1", "--prompt", prompt])?;
    }
    fixture.success(&[
        "workflow",
        "set-base",
        "--workflow",
        "1",
        "--base-revision",
        "root()",
    ])?;
    let before = fixture.success(&["workflow", "graph", "--workflow", "1"])?;
    assert_eq!(before["connectivity"]["status"], "needs_adoption");
    let refused = fixture
        .command()
        .args([
            "workflow",
            "adopt-roots",
            "--workflow",
            "1",
            "--base-nodes",
            "1",
        ])
        .output()?;
    assert!(!refused.status.success());
    assert_eq!(
        fixture.success(&["workflow", "graph", "--workflow", "1"])?,
        before
    );
    let adopted = fixture.success(&[
        "workflow",
        "adopt-roots",
        "--workflow",
        "1",
        "--base-nodes",
        "1,2",
    ])?;
    assert_eq!(adopted["connectivity"]["status"], "rooted");
    assert_eq!(adopted["base_nodes"], json!([1, 2]));
    fixture.stop()?;
    assert_eq!(
        fixture.success(&["workflow", "graph", "--workflow", "1"])?,
        adopted
    );
    Ok(())
}

#[test]
fn workflow_base_cli_persists_exact_start_and_supports_explicit_alternatives() -> anyhow::Result<()>
{
    let fixture = Fixture::new()?;
    jj(fixture.directory.path(), &["git", "init", "project"])?;
    let project_path = fixture.directory.path().join("project");
    let project = project_path.to_str().expect("project path");
    fs::write(project_path.join("base.txt"), "Starting code\n")?;
    jj(&project_path, &["describe", "-m", "Starting code"])?;
    jj(&project_path, &["bookmark", "create", "main"])?;
    jj(&project_path, &["bookmark", "create", "feature/auth"])?;
    let commit = recorded_commit(&project_path)?;
    let based = fixture.success(&[
        "workflow",
        "create",
        "--project",
        project,
        "--name",
        "Based",
        "--base",
        "main",
    ])?;
    assert_eq!(based["base"]["revision"]["commit_id"], commit);
    assert_eq!(
        based["base"]["source"],
        json!({"kind":"bookmark", "name":"main"})
    );
    fixture.success(&[
        "workflow",
        "create",
        "--project",
        project,
        "--name",
        "Alternate",
        "--base",
        "feature/auth",
    ])?;
    let exact = fixture.success(&[
        "workflow",
        "create",
        "--project",
        project,
        "--name",
        "Exact",
        "--base-revision",
        &commit,
    ])?;
    assert_eq!(exact["base"]["revision"]["commit_id"], commit);
    assert_eq!(
        exact["base"]["source"],
        json!({"kind":"revision", "id":commit})
    );
    let legacy = fixture.success(&[
        "workflow",
        "create",
        "--project",
        project,
        "--name",
        "Legacy",
    ])?;
    assert!(legacy["base"].is_null());
    let selected =
        fixture.success(&["workflow", "set-base", "--workflow", "4", "--base", "main"])?;
    assert_eq!(selected["base"], based["base"]);
    jj(&project_path, &["new", "-m", "Main advances"])?;
    jj(&project_path, &["bookmark", "set", "main", "-r", "@"])?;
    fixture.stop()?;
    assert_eq!(
        fixture.success(&["workflow", "get", "--workflow", "1"])?,
        based
    );
    assert_eq!(
        fixture.success(&["workflow", "graph", "--workflow", "1"])?["workflow"],
        based
    );
    assert_eq!(
        fixture.success(&["workflow", "list", "--project", project])?[0],
        based
    );
    let refused = fixture
        .command()
        .args(["workflow", "set-base", "--workflow", "1", "--base", "main"])
        .output()?;
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("already has a base"));
    let mut frame = serde_json::to_vec(&json!({
        "jsonrpc":"2.0", "id":1, "method":"workflow.create",
        "params":{"project_path":project,"name":"Malformed","base":{"kind":"bookmark","name":"main","force":true}}
    }))?;
    frame.push(b'\n');
    assert_eq!(fixture.rpc(&frame)?["error"]["code"], -32602);
    for args in [
        vec![
            "workflow",
            "create",
            "--project",
            project,
            "--name",
            "Missing",
            "--base",
            "missing",
        ],
        vec![
            "workflow",
            "create",
            "--project",
            project,
            "--name",
            "Both",
            "--base",
            "main",
            "--base-revision",
            &commit,
        ],
        vec!["workflow", "set-base", "--workflow", "1"],
    ] {
        assert!(!fixture.command().args(args).output()?.status.success());
    }
    assert_eq!(
        fixture
            .success(&["workflow", "list", "--project", project])?
            .as_array()
            .expect("workflows")
            .len(),
        4
    );
    Ok(())
}

#[test]
fn metadata_history_is_shared_persistent_and_rejects_stale_cli_and_rpc_actions()
-> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let project = fixture.directory.path().to_str().expect("temporary path");
    fixture.success(&[
        "workflow",
        "create",
        "--project",
        project,
        "--name",
        "First",
    ])?;
    fixture.success(&[
        "workflow",
        "create",
        "--project",
        project,
        "--name",
        "Second",
    ])?;
    fixture.success(&["node", "create", "--workflow", "1", "--prompt", "Original"])?;
    fixture.success(&[
        "node",
        "create",
        "--workflow",
        "2",
        "--prompt",
        "Other workflow",
    ])?;
    let pending = fixture.success(&["history", "status", "--project", project])?;
    assert_eq!(pending["undo"]["workflow_id"], 2);
    let id = pending["undo"]["id"].to_string();
    fixture.success(&["history", "undo", "--project", project, "--expected", &id])?;
    assert_eq!(
        fixture.success(&["workflow", "graph", "--workflow", "2"])?["nodes"],
        json!([])
    );
    assert_eq!(
        fixture.success(&["node", "get", "--workflow", "1", "--node", "1"])?["prompt"],
        "Original"
    );
    fixture.stop()?;
    let reopened = fixture.success(&["history", "status", "--project", project])?;
    assert_eq!(reopened["redo"], pending["undo"]);
    let mut frame = serde_json::to_vec(&json!({
        "jsonrpc": "2.0", "id": 1, "method": "history.redo",
        "params": {"project_path": project, "expected_edit_id": pending["undo"]["id"]}
    }))?;
    frame.push(b'\n');
    let redo = fixture.rpc(&frame)?;
    assert_eq!(redo["result"], pending);
    assert_eq!(
        fixture.success(&["node", "get", "--workflow", "2", "--node", "2"])?["prompt"],
        "Other workflow"
    );
    fixture.success(&[
        "node",
        "update",
        "--workflow",
        "1",
        "--node",
        "1",
        "--prompt",
        "Newer edit",
    ])?;
    let stale = fixture
        .command()
        .args(["history", "undo", "--project", project, "--expected", &id])
        .output()?;
    assert!(!stale.status.success());
    assert!(String::from_utf8_lossy(&stale.stderr).contains("history changed"));
    for params in [
        json!({"project_path": project}),
        json!({"project_path": project, "expected_edit_id": 1, "force": true}),
    ] {
        let mut frame = serde_json::to_vec(
            &json!({"jsonrpc":"2.0", "id":2, "method":"history.undo", "params":params}),
        )?;
        frame.push(b'\n');
        assert_eq!(fixture.rpc(&frame)?["error"]["code"], -32602);
    }
    assert_eq!(
        fixture.success(&["node", "get", "--workflow", "1", "--node", "1"])?["prompt"],
        "Newer edit"
    );
    Ok(())
}

#[test]
fn metadata_undo_never_rewinds_code_runs_or_conversation_and_blocks_referenced_creation()
-> anyhow::Result<()> {
    let (fixture, agent, workspace) = run_fixture("permission")?;
    let project_path = fixture.directory.path().join("project");
    let project = project_path.to_str().expect("project path");
    let creation = fixture.success(&["history", "status", "--project", project])?;
    let create_id = creation["undo"]["id"].to_string();
    fs::write(workspace.join("manual.txt"), "Keep these dirty edits\n")?;
    let commit = recorded_commit(&workspace)?;
    let operation = jj(
        &workspace,
        &[
            "--ignore-working-copy",
            "--at-op=@",
            "op",
            "log",
            "--limit",
            "1",
            "--no-graph",
            "-T",
            "id",
        ],
    )?;
    let checkout = fs::read(workspace.join(".jj/working_copy/tree_state"))?;
    let started = start_run(&fixture, &agent)?;
    let id = started["run"]["id"].to_string();
    let waiting = wait_run(&fixture, &id, "awaiting_permission")?;
    fixture.success(&[
        "node",
        "update",
        "--workflow",
        "1",
        "--node",
        "1",
        "--prompt",
        "Revised saved assignment",
    ])?;
    let edited = fixture.success(&["history", "status", "--project", project])?;
    let edit_id = edited["undo"]["id"].to_string();
    let requests = agent_requests(&agent)?;
    fixture.success(&[
        "history",
        "undo",
        "--project",
        project,
        "--expected",
        &edit_id,
    ])?;
    assert_eq!(
        fixture.success(&["node", "get", "--workflow", "1", "--node", "1"])?["prompt"],
        "permission"
    );
    fixture.success(&[
        "history",
        "redo",
        "--project",
        project,
        "--expected",
        &edit_id,
    ])?;
    assert_eq!(fixture.success(&["run", "status", "--run", &id])?, waiting);
    assert_eq!(agent_requests(&agent)?, requests);
    assert_eq!(
        fs::read_to_string(workspace.join("manual.txt"))?,
        "Keep these dirty edits\n"
    );
    assert_eq!(recorded_commit(&workspace)?, commit);
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
                "--no-graph",
                "-T",
                "id"
            ]
        )?,
        operation
    );
    assert_eq!(
        fs::read(workspace.join(".jj/working_copy/tree_state"))?,
        checkout
    );
    fixture.success(&[
        "history",
        "undo",
        "--project",
        project,
        "--expected",
        &edit_id,
    ])?;
    let before = fixture.success(&["history", "status", "--project", project])?;
    let blocked = fixture
        .command()
        .args([
            "history",
            "undo",
            "--project",
            project,
            "--expected",
            &create_id,
        ])
        .output()?;
    assert!(!blocked.status.success());
    assert!(String::from_utf8_lossy(&blocked.stderr).contains("references depend on it"));
    assert_eq!(
        fixture.success(&["history", "status", "--project", project])?,
        before
    );
    assert_eq!(fixture.success(&["run", "status", "--run", &id])?, waiting);
    fixture.success(&["run", "cancel", "--run", &id])?;
    wait_run(&fixture, &id, "cancelled")?;
    let blocked = fixture
        .command()
        .args([
            "history",
            "undo",
            "--project",
            project,
            "--expected",
            &create_id,
        ])
        .output()?;
    assert!(
        !blocked.status.success(),
        "even stopped run/native references must be retained"
    );
    Ok(())
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
fn immediate_restarts_preserve_graph_across_repeated_shutdowns() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let project = fixture.directory.path().to_str().expect("temporary path");
    fixture.success(&[
        "workflow",
        "create",
        "--project",
        project,
        "--name",
        "Restart perimeter",
    ])?;
    for prompt in ["Original assignment", "Dependent assignment"] {
        fixture.success(&["node", "create", "--workflow", "1", "--prompt", prompt])?;
    }
    fixture.success(&[
        "connection",
        "create",
        "--workflow",
        "1",
        "--source",
        "1",
        "--target",
        "2",
    ])?;
    let expected = fixture.success(&["workflow", "graph", "--workflow", "1"])?;
    let mut previous = fixture.success(&["backend", "status"])?;

    // No retry or settling delay after socket removal: exercise the same handoff
    // as the intermittent graph test, repeatedly within one test invocation.
    for cycle in 1..=20 {
        fixture.stop()?;
        let output = fixture
            .command()
            .args(["workflow", "graph", "--workflow", "1"])
            .output()?;
        let graph = fixture.json_output(output, &format!("immediate restart cycle {cycle}"))?;
        assert_eq!(graph, expected, "saved graph changed on cycle {cycle}");
        let current = fixture.success(&["backend", "status"])?;
        assert_ne!(
            current["pid"], previous["pid"],
            "cycle {cycle} did not restart"
        );
        assert_eq!(current, fixture.success(&["backend", "status"])?);
        previous = current;
    }
    Ok(())
}

#[test]
fn simultaneous_restarts_preserve_saved_workflow() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let output = fixture
        .command()
        .args([
            "workflow",
            "create",
            "--name",
            "Concurrent restarts",
            "--project",
        ])
        .arg(fixture.directory.path())
        .output()?;
    let saved = fixture.json_output(output, "create workflow")?;
    let mut previous = fixture.success(&["backend", "status"])?;

    for cycle in 1..=8 {
        fixture.stop()?;
        let mut children = Vec::new();
        for _ in 0..4 {
            children.push(
                fixture
                    .list_command()
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()?,
            );
        }
        // Collect every child before asserting so a failed starter cannot leave
        // other CLI processes running against a fixture being removed.
        let outputs = children
            .into_iter()
            .map(Child::wait_with_output)
            .collect::<std::io::Result<Vec<_>>>()?;
        for (client, output) in outputs.into_iter().enumerate() {
            let list = fixture.json_output(
                output,
                &format!("simultaneous restart cycle {cycle}, client {client}"),
            )?;
            assert_eq!(
                list,
                json!([saved.clone()]),
                "cycle {cycle}, client {client}"
            );
        }
        let current = fixture.success(&["backend", "status"])?;
        assert_ne!(
            current["pid"], previous["pid"],
            "cycle {cycle} did not restart"
        );
        assert_eq!(current, fixture.success(&["backend", "status"])?);
        previous = current;
    }
    Ok(())
}

#[test]
fn unavailable_owner_is_not_bypassed_and_startup_works_after_release() -> anyhow::Result<()> {
    use std::os::unix::fs::MetadataExt;

    for lock_name in ["workflow.db.lock", "backend.sock.lock"] {
        let fixture = Fixture::new()?;
        let lock_path = fixture.directory.path().join(lock_name);
        let owner = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)?;
        owner.try_lock()?;
        let inode = owner.metadata()?.ino();
        // Model a still-owned database/socket with no listener available yet.
        // Startup must fail rather than replace the lock or open the database.
        let output = fixture.list_command().output()?;
        assert!(
            !output.status.success(),
            "{lock_name}: startup bypassed owner"
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("backend did not become ready"),
            "{lock_name}: unexpected failure: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            fixture
                .startup_diagnostics()
                .contains("backend ownership unavailable"),
            "{}",
            fixture.startup_diagnostics()
        );
        assert!(
            !fixture.database.exists(),
            "{lock_name}: database was opened"
        );
        assert!(!fixture.socket.exists(), "{lock_name}: socket was bound");
        assert_eq!(
            fs::metadata(&lock_path)?.ino(),
            inode,
            "{lock_name}: lock replaced"
        );
        drop(owner);

        fixture.start()?;
        assert_eq!(
            fs::metadata(&lock_path)?.ino(),
            inode,
            "{lock_name}: lock replaced"
        );
        fixture.stop()?;
    }
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
