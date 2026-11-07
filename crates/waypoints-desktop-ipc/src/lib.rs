//! Local control channel between the `waypoints` CLI and the desktop app.
//!
//! This is deliberately separate from the workflow backend JSON-RPC socket. It
//! only carries visual requests such as "open this project and workflow"; it
//! never creates, mutates, or runs workflow state.

use std::env;
use std::io::{self, BufRead, BufReader, ErrorKind, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Environment override for the desktop control socket path.
pub const SOCKET_ENV: &str = "WAYPOINTS_APP_SOCKET";
/// Default socket filename under the platform state directory.
pub const SOCKET_NAME: &str = "desktop.sock";
/// Largest request or response frame, including its newline.
pub const MAX_FRAME: usize = 64 * 1024;

/// A visual request from the CLI to a running desktop app.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    /// Canonical or user-supplied project directory. Omitted to focus only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<PathBuf>,
    /// Exact workflow database identity to reveal. Requires `project`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow: Option<i64>,
}

impl Request {
    pub fn open(project: PathBuf, workflow: Option<i64>) -> Self {
        Self {
            project: Some(project),
            workflow,
        }
    }

    pub fn focus() -> Self {
        Self::default()
    }
}

/// Result of a control request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenStatus {
    /// An existing app instance accepted the request.
    Focused,
    /// The CLI started a new app and it accepted the request.
    Launched,
}

impl OpenStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Focused => "focused",
            Self::Launched => "launched",
        }
    }
}

/// A control response. Application failures are reported in `error`; transport
/// failures use `io::Error` instead.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<OpenStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Response {
    pub fn focused(request: &Request) -> Self {
        Self::accepted(OpenStatus::Focused, request)
    }

    pub fn launched(request: &Request) -> Self {
        Self::accepted(OpenStatus::Launched, request)
    }

    fn accepted(status: OpenStatus, request: &Request) -> Self {
        Self {
            ok: true,
            status: Some(status),
            project: request.project.clone(),
            workflow: request.workflow,
            pid: Some(std::process::id()),
            error: None,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            status: None,
            project: None,
            workflow: None,
            pid: None,
            error: Some(message.into()),
        }
    }
}

/// Resolves the control socket from `WAYPOINTS_APP_SOCKET`, else `state_dir`.
///
/// # Errors
///
/// Returns an error when the override is empty or relative.
pub fn socket_path(state_dir: &Path) -> io::Result<PathBuf> {
    if let Some(raw) = env::var_os(SOCKET_ENV) {
        let path = PathBuf::from(raw);
        if path.as_os_str().is_empty() {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                format!("{SOCKET_ENV} is empty"),
            ));
        }
        if !path.is_absolute() {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                format!("{SOCKET_ENV} must be an absolute path"),
            ));
        }
        return Ok(path);
    }
    Ok(state_dir.join(SOCKET_NAME))
}

/// Sends one request and reads the app's response.
///
/// # Errors
///
/// Returns connection, timeout, framing, or JSON errors. A refused or missing
/// socket means no app is listening; callers may then launch one.
pub fn send(path: &Path, request: &Request) -> io::Result<Response> {
    let mut stream = UnixStream::connect(path)?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    write_frame(&mut stream, request)?;
    stream.shutdown(Shutdown::Write)?;
    let mut reader = BufReader::new(stream);
    let response = read_frame(&mut reader)?
        .ok_or_else(|| io::Error::new(ErrorKind::UnexpectedEof, "app closed without a response"))?;
    Ok(response)
}

/// Whether a connect error means no app is listening at the socket.
pub fn is_unavailable(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::NotFound | ErrorKind::ConnectionRefused
    )
}

/// Reads one newline-delimited request. `Ok(None)` is a clean end of stream.
///
/// # Errors
///
/// Returns an error for oversized or malformed frames.
pub fn read_request(reader: &mut impl BufRead) -> io::Result<Option<Request>> {
    read_frame(reader)
}

/// Reads one newline-delimited response. `Ok(None)` is a clean end of stream.
///
/// # Errors
///
/// Returns an error for oversized or malformed frames.
pub fn read_response(reader: &mut impl BufRead) -> io::Result<Option<Response>> {
    read_frame(reader)
}

/// Writes one newline-delimited request.
pub fn write_request(writer: &mut impl Write, request: &Request) -> io::Result<()> {
    write_frame(writer, request)
}

/// Writes one newline-delimited response.
pub fn write_response(writer: &mut impl Write, response: &Response) -> io::Result<()> {
    write_frame(writer, response)
}

fn read_frame<T: for<'de> Deserialize<'de>>(reader: &mut impl BufRead) -> io::Result<Option<T>> {
    let mut line = Vec::new();
    let read = reader
        .by_ref()
        .take((MAX_FRAME + 1) as u64)
        .read_until(b'\n', &mut line)?;
    if read == 0 {
        return Ok(None);
    }
    if line.len() > MAX_FRAME {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "desktop control frame exceeds 64 KiB",
        ));
    }
    if line.last() != Some(&b'\n') {
        return Err(io::Error::new(
            ErrorKind::UnexpectedEof,
            "desktop control frame is not newline terminated",
        ));
    }
    serde_json::from_slice(&line)
        .map(Some)
        .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))
}

fn write_frame<T: Serialize>(writer: &mut impl Write, value: &T) -> io::Result<()> {
    let mut line =
        serde_json::to_vec(value).map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
    if line.len() + 1 > MAX_FRAME {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "desktop control frame exceeds 64 KiB",
        ));
    }
    line.push(b'\n');
    writer.write_all(&line)?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixListener;
    use std::thread;

    use super::*;

    #[test]
    fn request_and_response_round_trip_over_a_socket() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join(SOCKET_NAME);
        let listener = UnixListener::bind(&path).expect("bind test socket");
        let received = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let request = read_request(&mut reader).expect("read").expect("request");
            assert_eq!(
                request,
                Request::open(PathBuf::from("/tmp/example"), Some(7))
            );
            let mut writer = stream;
            write_response(&mut writer, &Response::focused(&request)).expect("write");
        });

        let response = send(
            &path,
            &Request::open(PathBuf::from("/tmp/example"), Some(7)),
        )
        .expect("control round trip");
        assert!(response.ok);
        assert_eq!(response.status, Some(OpenStatus::Focused));
        assert_eq!(response.workflow, Some(7));
        received.join().expect("listener thread");
    }

    #[test]
    fn oversized_frames_are_rejected() {
        let mut input = vec![b'x'; MAX_FRAME + 2];
        input.push(b'\n');
        let error = read_request(&mut input.as_slice()).expect_err("oversized");
        assert_eq!(error.kind(), ErrorKind::InvalidData);
    }

    #[test]
    fn socket_override_must_be_absolute() {
        // The process environment is shared by tests; use a unique temporary value
        // and restore it even if the assertion fails.
        let previous = env::var_os(SOCKET_ENV);
        unsafe { env::set_var(SOCKET_ENV, "relative.sock") };
        let error = socket_path(Path::new("/tmp")).expect_err("relative override");
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
        match previous {
            Some(value) => unsafe { env::set_var(SOCKET_ENV, value) },
            None => unsafe { env::remove_var(SOCKET_ENV) },
        }
    }
}
