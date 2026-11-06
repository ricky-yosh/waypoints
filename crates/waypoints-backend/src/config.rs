use std::fs::{self, DirBuilder};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::BackendError;

/// Absolute database and socket locations used by both clients and the backend.
#[derive(Debug, Clone)]
pub struct Config {
    pub(crate) database: PathBuf,
    pub(crate) socket: PathBuf,
}

impl Config {
    /// Resolves paths and creates missing parent directories with private permissions.
    ///
    /// Existing database symlinks are resolved to coordinate ownership by target.
    ///
    /// # Errors
    ///
    /// Returns an error if a parent cannot be created or a path cannot be resolved.
    pub fn new(database: &Path, socket: &Path) -> Result<Self, BackendError> {
        let database = absolute_file(database)?;
        let database = if database.exists() {
            fs::canonicalize(&database).context("resolving database path")?
        } else {
            database
        };
        let socket = absolute_file(socket)?;
        std::os::unix::net::SocketAddr::from_pathname(&socket)
            .context("invalid Unix socket path; supply a shorter --socket path")?;
        if database == socket {
            return Err(anyhow::anyhow!("database and socket paths must differ").into());
        }
        Ok(Self { database, socket })
    }

    /// Returns the resolved database filename.
    pub fn database(&self) -> &Path {
        &self.database
    }

    /// Returns the resolved Unix socket filename.
    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// Builds the default database and sibling socket under the platform state directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the state directory cannot be resolved or the paths are invalid.
    pub fn default_paths() -> Result<Self, BackendError> {
        let database = default_state_directory()?.join("workflows.db");
        Self::new(&database, &database.with_extension("sock"))
    }
}

/// Resolves the platform state directory that holds the default database.
///
/// Uses `XDG_STATE_HOME` when set, otherwise `~/Library/Application Support` on
/// macOS and `~/.local/state` elsewhere.
///
/// # Errors
///
/// Returns an error if `XDG_STATE_HOME` is relative or `HOME` is unavailable.
pub fn default_state_directory() -> Result<PathBuf, BackendError> {
    if let Some(path) = std::env::var_os("XDG_STATE_HOME").filter(|path| !path.is_empty()) {
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err(anyhow::anyhow!("XDG_STATE_HOME must be absolute").into());
        }
        return Ok(path.join("waypoints"));
    }
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?);
    #[cfg(target_os = "macos")]
    let path = home.join("Library/Application Support/waypoints");
    #[cfg(not(target_os = "macos"))]
    let path = home.join(".local/state/waypoints");
    Ok(path)
}

fn absolute_file(path: &Path) -> anyhow::Result<PathBuf> {
    let name = path.file_name().context("path must name a file")?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)
        .with_context(|| format!("creating directory {}", parent.display()))?;
    Ok(fs::canonicalize(parent)
        .with_context(|| format!("resolving directory {}", parent.display()))?
        .join(name))
}

pub(crate) fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}
