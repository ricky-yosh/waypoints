//! App-local workspace preferences, independent of project and backend storage.
//!
//! The file is `waypoints/workspace.json` under the platform configuration
//! directory: `$XDG_CONFIG_HOME` (or `~/.config`) on Linux, the user's
//! Application Support directory on macOS, and Roaming AppData on Windows.
//! Set `WAYPOINTS_WORKSPACE_FILE` to an absolute path for an isolated workspace.
//! Loading and saving perform synchronous filesystem I/O; call them off the UI thread.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

const VERSION: u64 = 1;
const DEFAULT_SIDEBAR_WIDTH: f32 = 240.0;
pub(crate) const MIN_SIDEBAR_WIDTH: f32 = 160.0;
pub(crate) const MAX_SIDEBAR_WIDTH: f32 = 1000.0;
const MIN_WINDOW_SIZE: f32 = 100.0;
const MAX_WINDOW_SIZE: f32 = 16384.0;
const MAX_WINDOW_POSITION: f32 = 100_000.0;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct WorkspacePreferences {
    #[serde(default)]
    pub(crate) graph_height: Option<f32>,
    #[serde(default)]
    pub(crate) graph_collapsed: bool,
    #[serde(default)]
    pub(crate) appearance: Appearance,
    pub(crate) projects: Vec<ProjectPreferences>,
    pub(crate) selected: Option<SelectedWorkflow>,
    pub(crate) sidebar_width: f32,
    pub(crate) sidebar_visible: bool,
    pub(crate) window_bounds: Option<WindowGeometry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ProjectPreferences {
    #[serde(with = "path_encoding")]
    pub(crate) path: PathBuf,
    pub(crate) expanded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SelectedWorkflow {
    #[serde(with = "path_encoding")]
    pub(crate) project: PathBuf,
    pub(crate) workflow_id: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) struct WindowGeometry {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
}

impl Default for WorkspacePreferences {
    fn default() -> Self {
        Self {
            appearance: Appearance::System,
            projects: Vec::new(),
            selected: None,
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
            sidebar_visible: true,
            graph_height: None,
            graph_collapsed: false,
            window_bounds: None,
        }
    }
}

impl WorkspacePreferences {
    fn sanitize(&mut self) {
        if !self.sidebar_width.is_finite()
            || !(MIN_SIDEBAR_WIDTH..=MAX_SIDEBAR_WIDTH).contains(&self.sidebar_width)
        {
            self.sidebar_width = DEFAULT_SIDEBAR_WIDTH;
        }
        if self.window_bounds.is_some_and(|bounds| {
            !bounds.x.is_finite()
                || !bounds.y.is_finite()
                || bounds.x.abs() > MAX_WINDOW_POSITION
                || bounds.y.abs() > MAX_WINDOW_POSITION
                || !bounds.width.is_finite()
                || !bounds.height.is_finite()
                || !(MIN_WINDOW_SIZE..=MAX_WINDOW_SIZE).contains(&bounds.width)
                || !(MIN_WINDOW_SIZE..=MAX_WINDOW_SIZE).contains(&bounds.height)
        }) {
            self.window_bounds = None;
        }
    }
}

#[derive(Serialize, Deserialize)]
struct PreferencesFile {
    version: u64,
    #[serde(flatten)]
    workspace: WorkspacePreferences,
}

// UTF-8 paths remain readable; non-UTF-8 paths use lossless platform-specific units.
mod path_encoding {
    use std::path::{Path, PathBuf};

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    #[serde(untagged)]
    enum EncodedPath {
        Text(String),
        Unix { unix_bytes: Vec<u8> },
        Windows { windows_wide: Vec<u16> },
    }

    pub(super) fn serialize<S: Serializer>(path: &Path, serializer: S) -> Result<S::Ok, S::Error> {
        if let Some(text) = path.to_str() {
            return EncodedPath::Text(text.to_owned()).serialize(serializer);
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            EncodedPath::Unix {
                unix_bytes: path.as_os_str().as_bytes().to_vec(),
            }
            .serialize(serializer)
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            EncodedPath::Windows {
                windows_wide: path.as_os_str().encode_wide().collect(),
            }
            .serialize(serializer)
        }
        #[cfg(not(any(unix, windows)))]
        {
            Err(serde::ser::Error::custom(
                "non-UTF-8 paths are unsupported on this platform",
            ))
        }
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<PathBuf, D::Error> {
        match EncodedPath::deserialize(deserializer)? {
            EncodedPath::Text(text) => Ok(text.into()),
            EncodedPath::Unix { unix_bytes } => {
                #[cfg(unix)]
                {
                    use std::os::unix::ffi::OsStringExt;
                    Ok(std::ffi::OsString::from_vec(unix_bytes).into())
                }
                #[cfg(not(unix))]
                {
                    let _ = unix_bytes;
                    Err(serde::de::Error::custom(
                        "Unix path cannot be decoded on this platform",
                    ))
                }
            }
            EncodedPath::Windows { windows_wide } => {
                #[cfg(windows)]
                {
                    use std::os::windows::ffi::OsStringExt;
                    Ok(std::ffi::OsString::from_wide(&windows_wide).into())
                }
                #[cfg(not(windows))]
                {
                    let _ = windows_wide;
                    Err(serde::de::Error::custom(
                        "Windows path cannot be decoded on this platform",
                    ))
                }
            }
        }
    }
}

#[derive(Deserialize)]
struct VersionHeader {
    version: u64,
}

fn preferences_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("WAYPOINTS_WORKSPACE_FILE") {
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            bail!("WAYPOINTS_WORKSPACE_FILE must be absolute");
        }
        return Ok(path);
    }

    let base = dirs::config_dir().context("cannot determine the user configuration directory")?;
    Ok(base.join("waypoints").join("workspace.json"))
}

/// Loads preferences, returning defaults only when the file is absent.
pub(crate) fn load_preferences() -> Result<WorkspacePreferences> {
    load_from(&preferences_path()?)
}

/// Atomically replaces preferences, refusing to overwrite unreadable or unsupported files.
pub(crate) fn save_preferences(preferences: &WorkspacePreferences) -> Result<()> {
    save_to(&preferences_path()?, preferences)
}

fn load_from(path: &Path) -> Result<WorkspacePreferences> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(WorkspacePreferences::default());
        }
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    // Inspect the version before decoding the rest so future formats get a useful error.
    let header: VersionHeader = serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "malformed preferences at {}; repair or move the file",
            path.display()
        )
    })?;
    let version = header.version;
    if version != VERSION {
        bail!(
            "unsupported preferences version {version} in {}; use a compatible Waypoints version or move the file",
            path.display()
        );
    }
    let mut file: PreferencesFile = serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "malformed preferences at {}; repair or move the file",
            path.display()
        )
    })?;
    file.workspace.sanitize();
    Ok(file.workspace)
}

fn save_to(path: &Path, preferences: &WorkspacePreferences) -> Result<()> {
    let parent = path
        .parent()
        .context("preferences path has no parent directory")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("creating preferences directory {}", parent.display()))?;
    // Lock a separate, permanent inode: replacing the preferences file must not
    // let another cooperating process acquire a lock on a different inode.
    let lock_path = path.with_extension("lock");
    let lock = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("opening preferences lock {}", lock_path.display()))?;
    lock.lock()
        .with_context(|| format!("locking preferences at {}", lock_path.display()))?;
    // A fallback to defaults after a load failure must not destroy the original file.
    load_from(path)?;
    let mut workspace = preferences.clone();
    workspace.sanitize();
    let bytes = serde_json::to_vec_pretty(&PreferencesFile {
        version: VERSION,
        workspace,
    })
    .context("serializing workspace preferences")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("creating temporary preferences in {}", parent.display()))?;
    temporary
        .write_all(&bytes)
        .context("writing temporary preferences")?;
    temporary
        .write_all(b"\n")
        .context("writing temporary preferences")?;
    temporary
        .as_file()
        .sync_all()
        .context("syncing temporary preferences")?;
    temporary
        .persist(path)
        .with_context(|| format!("replacing preferences at {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_file_returns_defaults() -> Result<()> {
        let dir = tempfile::tempdir()?;
        assert_eq!(
            load_from(&dir.path().join("missing.json"))?,
            WorkspacePreferences::default()
        );
        Ok(())
    }

    #[test]
    fn round_trip_preserves_order_and_missing_paths() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("nested/workspace.json");
        let preferences = WorkspacePreferences {
            projects: vec![
                ProjectPreferences {
                    path: "missing-b".into(),
                    expanded: true,
                },
                ProjectPreferences {
                    path: "missing-a".into(),
                    expanded: false,
                },
            ],
            selected: Some(SelectedWorkflow {
                project: "missing-a".into(),
                workflow_id: 42,
            }),
            appearance: Appearance::Dark,
            sidebar_width: 350.0,
            sidebar_visible: false,
            graph_height: Some(210.0),
            graph_collapsed: true,
            window_bounds: Some(WindowGeometry {
                x: -400.0,
                y: 100.0,
                width: 1200.0,
                height: 800.0,
            }),
        };
        save_to(&path, &preferences)?;
        assert_eq!(load_from(&path)?, preferences);
        save_to(&path, &preferences)?;
        assert_eq!(load_from(&path)?, preferences);
        Ok(())
    }

    #[test]
    fn malformed_and_future_files_are_not_overwritten() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("workspace.json");
        for contents in [
            r#"{"version":1,"projects":"bad"}"#,
            r#"{"version":999,"new_format":true}"#,
            r#"{"version":1,"version":999,"projects":[],"selected":null,"sidebar_width":240,"sidebar_visible":true,"window_bounds":null}"#,
            r#"{"version":1,"projects":[],"projects":[],"selected":null,"sidebar_width":240,"sidebar_visible":true,"window_bounds":null}"#,
            "not json",
        ] {
            fs::write(&path, contents)?;
            assert!(load_from(&path).is_err(), "accepted {contents}");
            assert!(
                save_to(&path, &WorkspacePreferences::default()).is_err(),
                "overwrote {contents}"
            );
            assert_eq!(fs::read_to_string(&path)?, contents);
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_paths_round_trip() -> Result<()> {
        use std::os::unix::ffi::OsStringExt;

        let dir = tempfile::tempdir()?;
        let path = dir.path().join("workspace.json");
        let project = PathBuf::from(std::ffi::OsString::from_vec(b"missing-\xff".to_vec()));
        let preferences = WorkspacePreferences {
            projects: vec![ProjectPreferences {
                path: project.clone(),
                expanded: true,
            }],
            selected: Some(SelectedWorkflow {
                project,
                workflow_id: 7,
            }),
            ..WorkspacePreferences::default()
        };
        save_to(&path, &preferences)?;
        assert_eq!(load_from(&path)?, preferences);
        Ok(())
    }

    #[test]
    fn save_waits_for_stable_lock_then_checks_file() -> Result<()> {
        use std::sync::mpsc;
        use std::time::Duration;

        let dir = tempfile::tempdir()?;
        let path = dir.path().join("workspace.json");
        let lock = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(path.with_extension("lock"))?;
        lock.lock()?;
        let (started_tx, started_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let thread_path = path.clone();
        let thread = std::thread::spawn(move || {
            started_tx.send(()).expect("test receiver exists");
            done_tx
                .send(save_to(&thread_path, &WorkspacePreferences::default()))
                .expect("test receiver exists");
        });
        started_rx.recv()?;
        assert!(done_rx.recv_timeout(Duration::from_millis(100)).is_err());
        let future = r#"{"version":999}"#;
        fs::write(&path, future)?;
        drop(lock);
        assert!(done_rx.recv_timeout(Duration::from_secs(5))?.is_err());
        thread.join().expect("save thread completes");
        assert_eq!(fs::read_to_string(path)?, future);
        Ok(())
    }

    #[test]
    fn invalid_geometry_is_sanitized_on_save_and_load() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("workspace.json");
        let preferences = WorkspacePreferences {
            sidebar_width: f32::NAN,
            window_bounds: Some(WindowGeometry {
                x: 0.0,
                y: 0.0,
                width: -1.0,
                height: 800.0,
            }),
            ..WorkspacePreferences::default()
        };
        save_to(&path, &preferences)?;
        assert_eq!(load_from(&path)?, WorkspacePreferences::default());
        fs::write(
            &path,
            r#"{"version":1,"projects":[],"selected":null,"sidebar_width":100000,"sidebar_visible":true,"window_bounds":{"x":100001,"y":0,"width":800,"height":600}}"#,
        )?;
        assert_eq!(load_from(&path)?, WorkspacePreferences::default());
        Ok(())
    }
}
