use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::net::UnixStream;

pub(crate) const TIMEOUT: Duration = Duration::from_secs(5);
// Bound memory usage even when a client sends a frame without a newline.
pub(crate) const MAX_FRAME: usize = 1024 * 1024;
pub(crate) const PROTOCOL_VERSION: u32 = 1;

/// The identity returned by a running backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BackendInfo {
    /// The server process identifier.
    pub pid: u32,
    /// The canonical database path owned by the server.
    pub database: PathBuf,
    /// The application protocol version, separate from JSON-RPC 2.0.
    pub protocol_version: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProjectParams {
    pub project_path: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateParams {
    pub project_path: PathBuf,
    pub name: String,
}

pub(crate) async fn read_frame(stream: &mut BufReader<UnixStream>) -> anyhow::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    (&mut *stream)
        .take(MAX_FRAME as u64 + 1)
        .read_until(b'\n', &mut bytes)
        .await
        .context("reading JSON-RPC frame")?;
    anyhow::ensure!(bytes.len() <= MAX_FRAME, "JSON-RPC frame exceeds 1 MiB");
    anyhow::ensure!(bytes.last() == Some(&b'\n'), "incomplete JSON-RPC frame");
    Ok(bytes)
}
