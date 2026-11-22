//! Persistence: user settings in `$XDG_CONFIG_HOME/zedterm/settings.toml` and
//! window state (size, maximized) in `$XDG_STATE_HOME/zedterm/window.toml`.
//!
//! Failures never stop the app: a missing or broken file falls back to the
//! defaults (with a warning), and a failed write is reported and ignored.

use crate::settings::AppSettings;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// `$XDG_*_HOME/zedterm`, falling back to `$HOME/<fallback>/zedterm`.
fn app_dir(xdg_var: &str, fallback: &str) -> Option<PathBuf> {
    let base = std::env::var_os(xdg_var)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(fallback)))?;
    Some(base.join("zedterm"))
}

fn settings_path() -> Option<PathBuf> {
    Some(app_dir("XDG_CONFIG_HOME", ".config")?.join("settings.toml"))
}

fn window_state_path() -> Option<PathBuf> {
    Some(app_dir("XDG_STATE_HOME", ".local/state")?.join("window.toml"))
}

fn load<T: DeserializeOwned + Default>(path: Option<PathBuf>) -> T {
    let Some(path) = path else {
        return T::default();
    };
    match fs::read_to_string(&path) {
        Ok(text) => toml::from_str(&text).unwrap_or_else(|err| {
            eprintln!("zedterm: ignoring invalid {}: {err}", path.display());
            T::default()
        }),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => T::default(),
        Err(err) => {
            eprintln!("zedterm: cannot read {}: {err}", path.display());
            T::default()
        }
    }
}

fn save<T: Serialize>(path: Option<PathBuf>, value: &T) {
    let Some(path) = path else {
        return;
    };
    if let Err(err) = write_atomically(&path, value) {
        eprintln!("zedterm: cannot write {}: {err}", path.display());
    }
}

/// Write to a temporary file and rename it, so a crash mid-write can't leave
/// a truncated file behind.
fn write_atomically<T: Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, toml::to_string_pretty(value)?)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

pub fn load_settings() -> AppSettings {
    load::<AppSettings>(settings_path()).sanitized()
}

pub fn save_settings(settings: &AppSettings) {
    save(settings_path(), settings);
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct WindowState {
    pub width: f32,
    pub height: f32,
    pub maximized: bool,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            width: 800.0,
            height: 600.0,
            maximized: false,
        }
    }
}

impl WindowState {
    /// Keep a corrupted or hand-edited file from opening an unusable window.
    fn sanitized(self) -> Self {
        let valid = |v: f32| v.is_finite() && v >= 200.0;
        if valid(self.width) && valid(self.height) {
            self
        } else {
            Self {
                maximized: self.maximized,
                ..Self::default()
            }
        }
    }
}

pub fn load_window_state() -> WindowState {
    load::<WindowState>(window_state_path()).sanitized()
}

pub fn save_window_state(state: &WindowState) {
    save(window_state_path(), state);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip() {
        let settings = AppSettings {
            font_size: 18.0,
            background_opacity: 0.8,
            ..Default::default()
        };
        let text = toml::to_string_pretty(&settings).unwrap();
        let loaded: AppSettings = toml::from_str(&text).unwrap();
        assert_eq!(loaded.font_size, 18.0);
        assert_eq!(loaded.background_opacity, 0.8);
        assert_eq!(loaded.theme, settings.theme);
    }

    #[test]
    fn missing_keys_use_defaults() {
        let loaded: AppSettings = toml::from_str("font-size = 20.0").unwrap();
        assert_eq!(loaded.font_size, 20.0);
        assert_eq!(loaded.scrollback, AppSettings::default().scrollback);
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        let loaded: AppSettings =
            toml::from_str("font-size = 500.0\nbackground-opacity = 0.0").unwrap();
        let loaded = loaded.sanitized();
        assert_eq!(loaded.font_size, 72.0);
        assert_eq!(loaded.background_opacity, 0.1);
    }

    #[test]
    fn tiny_window_state_falls_back() {
        let state = WindowState {
            width: 3.0,
            height: f32::NAN,
            maximized: true,
        }
        .sanitized();
        assert_eq!(state.width, 800.0);
        assert!(state.maximized);
    }
}
