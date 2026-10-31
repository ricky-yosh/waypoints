use std::fs::OpenOptions;
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::Context;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::time::{Instant, sleep, timeout};
use waypoints_core::storage::Workflow;

use crate::config::sidecar;
use crate::protocol::{MAX_FRAME, PROTOCOL_VERSION, TIMEOUT, read_frame};
use crate::{BackendError, BackendInfo, Config};

/// A client that talks only to the local backend, never directly to its database.
#[derive(Debug)]
pub struct Client {
    config: Config,
    info: BackendInfo,
}

impl Client {
    /// Connects to an existing backend without starting one.
    ///
    /// # Errors
    ///
    /// Returns an error if connection, identity checks, or the readiness request fails.
    pub async fn connect(config: &Config) -> Result<Self, BackendError> {
        let stream = timeout(TIMEOUT, UnixStream::connect(&config.socket))
            .await
            .context("backend connection timed out")?
            .context("connecting to backend")?;
        Ok(Self::from_stream(config, stream).await?)
    }

    /// Reuses a backend or starts the supplied Waypoints executable in the background.
    ///
    /// Only missing or refused sockets trigger startup. A live but unresponsive or
    /// incompatible backend is reported as an error. Concurrent starters are safe:
    /// the server acquires ownership locks before opening the database.
    ///
    /// # Errors
    ///
    /// Returns an error if launching fails or the backend is not ready within five seconds.
    pub async fn connect_or_start(
        config: &Config,
        executable: &Path,
    ) -> Result<Self, BackendError> {
        match timeout(TIMEOUT, UnixStream::connect(&config.socket))
            .await
            .context("backend connection timed out")?
        {
            Ok(stream) => return Ok(Self::from_stream(config, stream).await?),
            Err(error) if unavailable(&error) => {}
            Err(error) => {
                return Err(anyhow::Error::new(error)
                    .context("connecting to backend")
                    .into());
            }
        }

        let log_path = sidecar(&config.socket, ".log");
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&log_path)
            .context("opening backend startup log")?;
        let mut child = Command::new(executable)
            .arg("--database")
            .arg(&config.database)
            .arg("--socket")
            .arg(&config.socket)
            .arg("serve")
            .current_dir("/")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log)
            .process_group(0)
            .spawn()
            .context("starting backend executable")?;
        let deadline = Instant::now() + TIMEOUT;
        loop {
            // Reap a losing starter if another process won the ownership lock.
            let _ = child.try_wait().context("checking backend startup")?;
            match UnixStream::connect(&config.socket).await {
                Ok(stream) => return Ok(Self::from_stream(config, stream).await?),
                Err(error) if unavailable(&error) => {}
                Err(error) => {
                    return Err(anyhow::Error::new(error)
                        .context("connecting to backend")
                        .into());
                }
            }
            if Instant::now() >= deadline {
                return Err(anyhow::anyhow!(
                    "backend did not become ready; inspect {} (another backend may own this database)",
                    log_path.display()
                ).into());
            }
            sleep(std::time::Duration::from_millis(25)).await;
        }
    }

    async fn from_stream(config: &Config, stream: UnixStream) -> anyhow::Result<Self> {
        let info: BackendInfo = exchange(stream, "backend.info", json!({})).await?;
        anyhow::ensure!(
            info.database == config.database && info.protocol_version == PROTOCOL_VERSION,
            "socket belongs to a different database or incompatible backend; refusing to start another server"
        );
        Ok(Self {
            config: config.clone(),
            info,
        })
    }

    /// Returns the identity checked when this client connected.
    pub fn info(&self) -> &BackendInfo {
        &self.info
    }

    /// Creates a workflow through the backend.
    ///
    /// # Errors
    ///
    /// Returns an error on invalid input, a server error, or a transport failure.
    pub async fn create_workflow(
        &self,
        project: &Path,
        name: &str,
    ) -> Result<Workflow, BackendError> {
        let project = project
            .canonicalize()
            .context("resolving project directory")?;
        self.call(
            "workflow.create",
            json!({"project_path": project, "name": name}),
        )
        .await
    }

    /// Lists a project's workflows through the backend.
    ///
    /// # Errors
    ///
    /// Returns an error on invalid input, a server error, or a transport failure.
    pub async fn list_workflows(&self, project: &Path) -> Result<Vec<Workflow>, BackendError> {
        let project = project
            .canonicalize()
            .context("resolving project directory")?;
        self.call("workflow.list", json!({"project_path": project}))
            .await
    }

    /// Requests orderly backend shutdown without deleting saved workflows.
    ///
    /// # Errors
    ///
    /// Returns an error if the shutdown request cannot be delivered or acknowledged.
    pub async fn stop(&self) -> Result<(), BackendError> {
        self.call("backend.shutdown", json!({})).await
    }

    async fn call<T: DeserializeOwned>(
        &self,
        method: &str,
        params: Value,
    ) -> Result<T, BackendError> {
        Ok(timeout(TIMEOUT, async {
            let stream = UnixStream::connect(&self.config.socket)
                .await
                .context("connecting to backend")?;
            exchange(stream, method, params).await
        })
        .await
        .context("backend request timed out")??)
    }
}

fn unavailable(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    )
}

async fn exchange<T: DeserializeOwned>(
    stream: UnixStream,
    method: &str,
    params: Value,
) -> anyhow::Result<T> {
    timeout(TIMEOUT, async {
        let mut stream = BufReader::new(stream);
        let mut request = serde_json::to_vec(&json!({
            "jsonrpc": "2.0", "id": 1, "method": method, "params": params
        }))?;
        request.push(b'\n');
        anyhow::ensure!(request.len() <= MAX_FRAME, "request exceeds 1 MiB");
        stream
            .get_mut()
            .write_all(&request)
            .await
            .context("sending backend request")?;
        let response: Value = serde_json::from_slice(&read_frame(&mut stream).await?)
            .context("decoding backend response")?;
        anyhow::ensure!(
            response["jsonrpc"] == "2.0" && response["id"] == 1,
            "invalid backend response"
        );
        if let Some(error) = response.get("error") {
            anyhow::bail!("backend error {}: {}", error["code"], error["message"]);
        }
        serde_json::from_value(
            response
                .get("result")
                .context("backend response has no result")?
                .clone(),
        )
        .context("decoding backend result")
    })
    .await
    .context("backend request timed out")?
}
