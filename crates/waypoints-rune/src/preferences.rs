//! Rune-only UI preferences, stored separately from the GPUI workspace file.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const VERSION: u64 = 1;
const MAX_WINDOW_SIZE: f32 = 16_384.0;
const MAX_WINDOW_POSITION: f32 = 100_000.0;
pub(crate) const INTERFACE_ZOOM_LEVELS: [f32; 4] = [0.85, 1.0, 1.15, 1.3];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    pub fn resolve(self, system_dark: bool) -> bool {
        match self {
            Self::System => system_dark,
            Self::Light => false,
            Self::Dark => true,
        }
    }

    pub fn index(self) -> usize {
        match self {
            Self::System => 0,
            Self::Light => 1,
            Self::Dark => 2,
        }
    }

    pub fn from_index(index: usize) -> Self {
        match index {
            1 => Self::Light,
            2 => Self::Dark,
            _ => Self::System,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) struct WindowGeometry {
    /// Content top-left in points from the primary display's top-left.
    pub x: f32,
    pub y: f32,
    /// Content size in points.
    pub width: f32,
    pub height: f32,
    /// Display scale used to turn `x`/`y` back into desktop pixels at launch.
    pub scale: f32,
}

impl WindowGeometry {
    pub fn valid(self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && self.width.is_finite()
            && self.height.is_finite()
            && self.scale.is_finite()
            && self.x.abs() <= MAX_WINDOW_POSITION
            && self.y.abs() <= MAX_WINDOW_POSITION
            && self.width >= crate::WINDOW_MIN_SIZE.0
            && self.height >= crate::WINDOW_MIN_SIZE.1
            && self.width <= MAX_WINDOW_SIZE
            && self.height <= MAX_WINDOW_SIZE
            && (0.5..=4.0).contains(&self.scale)
    }
}

/// Clamp saved content geometry to the first connected display it touches.
///
/// `display_bounds` are `[x, y, width, height]` in the same point space as the
/// saved geometry. Returns `None` when no display intersects it, so launch
/// falls back to the platform's centered default instead of restoring an
/// unreachable window.
pub(crate) fn clamp_window_geometry(
    geometry: WindowGeometry,
    display_bounds: &[[f32; 4]],
) -> Option<WindowGeometry> {
    if !geometry.valid() {
        return None;
    }
    let desired = [
        geometry.x,
        geometry.y,
        geometry.width.max(crate::WINDOW_MIN_SIZE.0),
        geometry.height.max(crate::WINDOW_MIN_SIZE.1),
    ];
    let display = display_bounds.iter().find(|bounds| {
        desired[0] < bounds[0] + bounds[2]
            && desired[0] + desired[2] > bounds[0]
            && desired[1] < bounds[1] + bounds[3]
            && desired[1] + desired[3] > bounds[1]
    })?;
    let width = desired[2]
        .max(crate::WINDOW_MIN_SIZE.0)
        .min(display[2].max(crate::WINDOW_MIN_SIZE.0));
    let height = desired[3]
        .max(crate::WINDOW_MIN_SIZE.1)
        .min(display[3].max(crate::WINDOW_MIN_SIZE.1));
    let left = display[0];
    let top = display[1];
    Some(WindowGeometry {
        x: desired[0].clamp(left, (left + display[2] - width).max(left)),
        y: desired[1].clamp(top, (top + display[3] - height).max(top)),
        width,
        height,
        scale: geometry.scale,
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct RunePreferences {
    pub appearance: Appearance,
    pub reduced_motion: bool,
    #[serde(default = "default_interface_zoom")]
    pub interface_zoom: f32,
    #[serde(default)]
    pub projects: Vec<PathBuf>,
    #[serde(default)]
    pub selected_project: Option<PathBuf>,
    #[serde(default)]
    pub selected_workflow: Option<i64>,
    #[serde(default = "default_sidebar_open")]
    pub sidebar_open: bool,
    #[serde(default)]
    pub window_geometry: Option<WindowGeometry>,
}

fn default_sidebar_open() -> bool {
    true
}

fn default_interface_zoom() -> f32 {
    1.0
}

impl Default for RunePreferences {
    fn default() -> Self {
        Self {
            appearance: Appearance::System,
            reduced_motion: false,
            interface_zoom: 1.0,
            projects: Vec::new(),
            selected_project: None,
            selected_workflow: None,
            sidebar_open: true,
            window_geometry: None,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct PreferencesFile {
    version: u64,
    #[serde(flatten)]
    preferences: RunePreferences,
}

pub(crate) struct Loaded {
    pub preferences: RunePreferences,
    pub error: Option<String>,
    pub writable: bool,
    pub path: Option<PathBuf>,
}

pub(crate) fn load() -> Loaded {
    let path = match preferences_path() {
        Ok(path) => path,
        Err(error) => {
            return Loaded {
                preferences: RunePreferences::default(),
                error: Some(error),
                writable: false,
                path: None,
            };
        }
    };
    load_path(&path)
}

fn load_path(path: &Path) -> Loaded {
    if !path.exists() {
        return Loaded {
            preferences: RunePreferences::default(),
            error: None,
            writable: true,
            path: Some(path.to_owned()),
        };
    }
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) => {
            return Loaded {
                preferences: RunePreferences::default(),
                error: Some(format!("Couldn’t read {}: {error}", path.display())),
                writable: false,
                path: Some(path.to_owned()),
            };
        }
    };
    let file: PreferencesFile = match serde_json::from_str(&text) {
        Ok(file) => file,
        Err(error) => {
            return Loaded {
                preferences: RunePreferences::default(),
                error: Some(format!(
                    "Couldn’t parse {}; the existing file was left untouched: {error}",
                    path.display()
                )),
                writable: false,
                path: Some(path.to_owned()),
            };
        }
    };
    if file.version > VERSION {
        return Loaded {
            preferences: RunePreferences::default(),
            error: Some(format!(
                "{} uses a newer preferences version {}; this build reads version {VERSION} and left it untouched.",
                path.display(),
                file.version
            )),
            writable: false,
            path: Some(path.to_owned()),
        };
    }
    let mut preferences = file.preferences;
    if !preferences.interface_zoom.is_finite()
        || !(INTERFACE_ZOOM_LEVELS[0]..=INTERFACE_ZOOM_LEVELS[INTERFACE_ZOOM_LEVELS.len() - 1])
            .contains(&preferences.interface_zoom)
    {
        preferences.interface_zoom = 1.0;
    }
    if preferences
        .window_geometry
        .is_some_and(|geometry| !geometry.valid())
    {
        preferences.window_geometry = None;
    }
    Loaded {
        preferences,
        error: None,
        writable: true,
        path: Some(path.to_owned()),
    }
}

pub(crate) fn save(preferences: &RunePreferences, path: &Path) -> Result<(), String> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .map_err(|error| format!("Couldn’t create {}: {error}", parent.display()))?;
    let file = PreferencesFile {
        version: VERSION,
        preferences: preferences.clone(),
    };
    let text = serde_json::to_string_pretty(&file)
        .map_err(|error| format!("Couldn’t encode Rune preferences: {error}"))?;
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, text)
        .map_err(|error| format!("Couldn’t write {}: {error}", temp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&temp, fs::Permissions::from_mode(0o600));
    }
    fs::rename(&temp, path).map_err(|error| format!("Couldn’t replace {}: {error}", path.display()))
}

fn preferences_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("WAYPOINTS_RUNE_PREFERENCES_FILE") {
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err("WAYPOINTS_RUNE_PREFERENCES_FILE must be an absolute path".into());
        }
        return Ok(path);
    }
    let directory = waypoints_backend::default_state_directory()
        .map_err(|error| format!("Couldn’t resolve the Rune preferences directory: {error}"))?;
    Ok(directory.join("rune-workspace.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appearance_resolves_and_round_trips() {
        assert!(!Appearance::Light.resolve(true));
        assert!(Appearance::Dark.resolve(false));
        assert!(Appearance::System.resolve(true));
        for appearance in [Appearance::System, Appearance::Light, Appearance::Dark] {
            assert_eq!(Appearance::from_index(appearance.index()), appearance);
        }
    }

    #[test]
    fn window_geometry_clamps_to_a_connected_display() {
        let display = [0.0, 0.0, 1440.0, 900.0];
        let partially_visible = WindowGeometry {
            x: 1400.0,
            y: -700.0,
            width: 1100.0,
            height: 800.0,
            scale: 2.0,
        };
        let clamped = clamp_window_geometry(partially_visible, &[display]).expect("visible");
        assert_eq!(
            (clamped.x, clamped.y, clamped.width, clamped.height),
            (340.0, 0.0, 1100.0, 800.0)
        );
        assert_eq!(
            clamp_window_geometry(
                WindowGeometry {
                    x: 3000.0,
                    ..partially_visible
                },
                &[display]
            ),
            None,
            "a disconnected display does not restore offscreen"
        );
        let fitted = clamp_window_geometry(
            WindowGeometry {
                x: 0.0,
                y: 0.0,
                width: 2000.0,
                height: 1400.0,
                scale: 2.0,
            },
            &[[0.0, 0.0, 1000.0, 700.0]],
        )
        .expect("fits the smaller display");
        assert_eq!((fitted.width, fitted.height), (1000.0, 700.0));
    }

    #[test]
    fn window_geometry_rejects_invalid_values() {
        let geometry = WindowGeometry {
            x: 40.0,
            y: 60.0,
            width: 1100.0,
            height: 760.0,
            scale: 2.0,
        };
        assert!(geometry.valid());
        assert!(
            !WindowGeometry {
                width: 10.0,
                ..geometry
            }
            .valid()
        );
        assert!(
            !WindowGeometry {
                scale: 0.0,
                ..geometry
            }
            .valid()
        );
        assert!(
            !WindowGeometry {
                x: f32::NAN,
                ..geometry
            }
            .valid()
        );
    }

    #[test]
    fn invalid_or_newer_files_are_not_writable() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("prefs.json");
        fs::write(&path, "{broken").expect("write invalid");
        let loaded = load_path(&path);
        assert!(loaded.error.is_some());
        assert!(!loaded.writable);
        fs::write(
            &path,
            r#"{"version": 99, "appearance": "dark", "reduced_motion": false}"#,
        )
        .expect("write newer");
        let loaded = load_path(&path);
        assert!(loaded.error.is_some());
        assert!(!loaded.writable);
    }

    #[test]
    fn preferences_save_and_load_round_trip() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("nested/prefs.json");
        let preferences = RunePreferences {
            appearance: Appearance::Dark,
            reduced_motion: true,
            interface_zoom: 1.15,
            projects: vec![PathBuf::from("/tmp/project")],
            selected_project: Some(PathBuf::from("/tmp/project")),
            selected_workflow: Some(7),
            sidebar_open: false,
            window_geometry: Some(WindowGeometry {
                x: 40.0,
                y: 60.0,
                width: 1100.0,
                height: 760.0,
                scale: 2.0,
            }),
        };
        save(&preferences, &path).expect("save preferences");
        let loaded = load_path(&path);
        assert!(loaded.error.is_none());
        assert!(loaded.writable);
        assert_eq!(loaded.preferences, preferences);
    }
}
