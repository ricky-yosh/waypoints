//! Agent-facing desktop open command tests: control-socket reuse and launcher fallback.

use std::io::BufReader;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

use serde_json::Value;
use tempfile::TempDir;
use waypoints_desktop_ipc::{Request, Response};

fn waypoints() -> Command {
    Command::new(env!("CARGO_BIN_EXE_waypoints"))
}

fn isolated_state() -> TempDir {
    tempfile::tempdir().expect("isolated state directory")
}

fn socket_path(state: &Path) -> PathBuf {
    state.join("waypoints").join("desktop.sock")
}

fn project(directory: &TempDir, name: &str) -> PathBuf {
    let project = directory.path().join(name);
    std::fs::create_dir(&project).expect("project directory");
    std::fs::canonicalize(project).expect("canonical project")
}

#[test]
fn app_open_reuses_a_running_desktop_and_validates_the_workflow() {
    let directory = isolated_state();
    let project = project(&directory, "project");
    let created = waypoints()
        .env("XDG_STATE_HOME", directory.path())
        .args([
            "workflow",
            "create",
            "--project",
            project.to_str().expect("utf-8 project"),
            "--name",
            "Agent workflow",
            "--json",
        ])
        .output()
        .expect("create workflow");
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let workflow: Value = serde_json::from_slice(&created.stdout).expect("workflow JSON");
    let workflow_id = workflow["id"].as_i64().expect("workflow id");

    let socket = socket_path(directory.path());
    std::fs::create_dir_all(socket.parent().expect("socket parent")).expect("socket dir");
    let listener = UnixListener::bind(&socket).expect("control socket");
    listener.set_nonblocking(false).expect("blocking listener");
    let expected_project = project.clone();
    let listener_thread = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept control request");
        let mut reader = BufReader::new(stream.try_clone().expect("clone"));
        let request: Request = waypoints_desktop_ipc::read_request(&mut reader)
            .expect("read request")
            .expect("request");
        assert_eq!(request.project.as_ref(), Some(&expected_project));
        assert_eq!(request.workflow, Some(workflow_id));
        let mut writer = stream;
        waypoints_desktop_ipc::write_response(&mut writer, &Response::focused(&request))
            .expect("response");
    });

    let opened = waypoints()
        .env("XDG_STATE_HOME", directory.path())
        .env("WAYPOINTS_APP_SOCKET", &socket)
        .args([
            "app",
            "open",
            "--project",
            project.to_str().expect("utf-8 project"),
            "--workflow",
            &workflow_id.to_string(),
            "--json",
        ])
        .output()
        .expect("open desktop");
    listener_thread.join().expect("listener thread");
    assert!(
        opened.status.success(),
        "{}",
        String::from_utf8_lossy(&opened.stderr)
    );
    let response: Value = serde_json::from_slice(&opened.stdout).expect("response JSON");
    assert_eq!(response["ok"], true);
    assert_eq!(response["status"], "focused");
    assert_eq!(response["workflow"], workflow_id);

    let listed = waypoints()
        .env("XDG_STATE_HOME", directory.path())
        .args([
            "workflow",
            "list",
            "--project",
            project.to_str().expect("utf-8 project"),
            "--json",
        ])
        .output()
        .expect("list workflows");
    assert!(
        listed.status.success(),
        "{}",
        String::from_utf8_lossy(&listed.stderr)
    );
    let workflows: Vec<Value> = serde_json::from_slice(&listed.stdout).expect("workflow list JSON");
    assert_eq!(
        workflows.len(),
        1,
        "opening the visual must not create or duplicate workflows"
    );
    assert_eq!(workflows[0]["id"], workflow_id);

    let stopped = waypoints()
        .env("XDG_STATE_HOME", directory.path())
        .args(["backend", "stop"])
        .output()
        .expect("stop backend");
    assert!(stopped.status.success());
}

#[test]
fn app_open_launches_the_desktop_when_no_socket_is_listening() {
    let directory = isolated_state();
    let project = project(&directory, "project");
    let socket = socket_path(directory.path());
    std::fs::create_dir_all(socket.parent().expect("socket parent")).expect("socket dir");
    let app = directory.path().join("fake-waypoints-app");
    let args_path = socket
        .parent()
        .expect("socket parent")
        .join("launched-args.json");
    std::fs::write(
        &app,
        r#"#!/usr/bin/env python3
import json, os, socket, sys
path = os.environ["WAYPOINTS_APP_SOCKET"]
with open(os.path.join(os.path.dirname(path), "launched-args.json"), "w") as f:
    json.dump(sys.argv[1:], f)
listener = socket.socket(socket.AF_UNIX)
listener.bind(path)
listener.listen(1)
conn, _ = listener.accept()
data = b""
while not data.endswith(b"\n"):
    data += conn.recv(4096)
request = json.loads(data)
response = {
    "ok": True,
    "status": "focused",
    "project": request.get("project"),
    "workflow": request.get("workflow"),
    "pid": os.getpid(),
}
conn.sendall(json.dumps(response).encode() + b"\n")
"#,
    )
    .expect("fake app");
    std::fs::set_permissions(&app, std::fs::Permissions::from_mode(0o700))
        .expect("executable permissions");

    let opened = waypoints()
        .env("XDG_STATE_HOME", directory.path())
        .env("WAYPOINTS_APP_SOCKET", &socket)
        .env("WAYPOINTS_APP_EXECUTABLE", &app)
        .args([
            "app",
            "open",
            "--project",
            project.to_str().expect("utf-8 project"),
            "--json",
        ])
        .output()
        .expect("open desktop");
    assert!(
        opened.status.success(),
        "{}",
        String::from_utf8_lossy(&opened.stderr)
    );
    let response: Value = serde_json::from_slice(&opened.stdout).expect("response JSON");
    assert_eq!(response["ok"], true);
    assert_eq!(response["status"], "launched");

    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while !args_path.exists() && std::time::Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    let args: Vec<String> =
        serde_json::from_str(&std::fs::read_to_string(&args_path).expect("launch args"))
            .expect("args JSON");
    assert_eq!(
        args,
        vec!["--project".to_owned(), project.display().to_string(),]
    );
}
