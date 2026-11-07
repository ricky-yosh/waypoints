//! Desktop control socket for agent-driven open/focus requests.
//!
//! The app binds a private Unix socket separate from the workflow backend. A
//! request only changes which project/workflow is visible and focuses the
//! window; it never creates or runs anything.

use std::fs;
use std::io::{self, BufReader};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use rune::AppHandle;
use waypoints_desktop_ipc::{Request, Response, SOCKET_ENV};

use crate::State;

/// Resolves the same control socket path the CLI uses.
pub(crate) fn socket_path() -> Result<PathBuf, String> {
    let state = waypoints_backend::default_state_directory()
        .map_err(|error| format!("Couldn’t resolve the desktop state directory: {error}"))?;
    waypoints_desktop_ipc::socket_path(&state).map_err(|error| {
        format!(
            "Couldn’t resolve {SOCKET_ENV} or {}/desktop.sock: {error}",
            state.display()
        )
    })
}

/// Sends the current launch's request to an already-running app.
///
/// Returns true when an existing app accepted it and this process should exit.
pub(crate) fn forward_if_running(path: &Path, request: &Request) -> bool {
    match waypoints_desktop_ipc::send(path, request) {
        Ok(response) if response.ok => {
            eprintln!("waypoints-app: focused the running desktop app");
            true
        }
        Ok(response) => {
            eprintln!(
                "waypoints-app: the running desktop app rejected the request: {}",
                response.error.unwrap_or_else(|| "unknown error".into())
            );
            false
        }
        Err(error) if waypoints_desktop_ipc::is_unavailable(&error) => false,
        Err(error) => {
            eprintln!("waypoints-app: desktop control request failed: {error}");
            false
        }
    }
}

/// Binds the control socket, clearing a stale socket left by a crashed app.
pub(crate) fn bind(path: &Path) -> io::Result<UnixListener> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
    }
    if path.exists() {
        match UnixStream::connect(path) {
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    "another desktop app is already listening",
                ));
            }
            Err(error) if waypoints_desktop_ipc::is_unavailable(&error) => {
                fs::remove_file(path)?;
            }
            Err(error) => return Err(error),
        }
    }
    let listener = UnixListener::bind(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// Serves open/focus requests until the process exits.
pub(crate) fn serve(listener: UnixListener, handle: AppHandle<State>) {
    thread::Builder::new()
        .name("waypoints-desktop-control".into())
        .spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let handle = handle.clone();
                        thread::spawn(move || handle_connection(stream, handle));
                    }
                    Err(error) => {
                        eprintln!("waypoints-app: desktop control accept failed: {error}")
                    }
                }
            }
        })
        .expect("spawning the desktop control listener");
}

fn handle_connection(stream: UnixStream, handle: AppHandle<State>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(clone) => clone,
        Err(error) => {
            eprintln!("waypoints-app: desktop control clone failed: {error}");
            return;
        }
    });
    let request = match waypoints_desktop_ipc::read_request(&mut reader) {
        Ok(Some(request)) => request,
        Ok(None) => return,
        Err(error) => {
            let mut writer = stream;
            let _ = waypoints_desktop_ipc::write_response(
                &mut writer,
                &Response::error(format!("invalid desktop request: {error}")),
            );
            return;
        }
    };
    let response = match prepare(&request) {
        Ok((project, workflow)) => {
            handle.update(move |state, _| {
                state.apply_open_request(project, workflow);
            });
            handle.focus();
            Response::focused(&request)
        }
        Err(error) => Response::error(error),
    };
    let mut writer = stream;
    if let Err(error) = waypoints_desktop_ipc::write_response(&mut writer, &response) {
        eprintln!("waypoints-app: desktop control response failed: {error}");
    }
}

fn prepare(request: &Request) -> Result<(Option<PathBuf>, Option<i64>), String> {
    if request.workflow.is_some() && request.project.is_none() {
        return Err("a workflow can only be opened with its project".into());
    }
    let project = match &request.project {
        Some(project) => {
            let project = project.canonicalize().map_err(|error| {
                format!(
                    "Couldn’t open project {}: {error}. Choose an existing folder.",
                    project.display()
                )
            })?;
            if !project.is_dir() {
                return Err(format!("{} is not a folder", project.display()));
            }
            Some(project)
        }
        None => None,
    };
    Ok((project, request.workflow))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_or_ambiguous_requests_are_rejected_before_state_changes() {
        let request = Request {
            project: None,
            workflow: Some(4),
        };
        assert!(prepare(&request).is_err());

        let directory = tempfile::tempdir().expect("temp dir");
        let file = directory.path().join("not-a-folder");
        fs::write(&file, "x").expect("file");
        let request = Request {
            project: Some(file),
            workflow: None,
        };
        assert!(prepare(&request).is_err());
    }
}
