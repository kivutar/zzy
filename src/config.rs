//! Persistence: user settings in `$XDG_CONFIG_HOME/zzy/settings.toml` and
//! window state (size, maximized) in `$XDG_STATE_HOME/zzy/window.toml`; on
//! Windows, in `%APPDATA%\zzy` and `%LOCALAPPDATA%\zzy`.
//!
//! Failures never stop the app: a missing or broken file falls back to the
//! defaults (with a warning), and a failed write is reported and ignored.

use crate::settings::AppSettings;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// `$XDG_*_HOME/zzy`, falling back to `$HOME/<fallback>/zzy`.
#[cfg(not(windows))]
fn app_dir(xdg_var: &str, fallback: &str) -> Option<PathBuf> {
    let base = std::env::var_os(xdg_var)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(fallback)))?;
    let dir = base.join("zzy");
    migrate_old_dir(&base.join("zedterm"), &dir);
    Some(dir)
}

/// The app used to be called zedterm: move its directory over once, so
/// existing settings and window state carry over. (zedterm never ran on
/// Windows, so only the XDG directories need it.)
#[cfg_attr(windows, allow(dead_code))]
fn migrate_old_dir(old: &Path, new: &Path) {
    if new.exists() || !old.is_dir() {
        return;
    }
    if let Err(err) = fs::rename(old, new) {
        eprintln!(
            "zzy: cannot move {} to {}: {err}",
            old.display(),
            new.display()
        );
    }
}

/// Windows has no `$HOME` or XDG directories: settings roam with the user
/// (`%APPDATA%`), window state stays on this machine (`%LOCALAPPDATA%`).
#[cfg(windows)]
fn app_dir(xdg_var: &str, _fallback: &str) -> Option<PathBuf> {
    let var = if xdg_var == "XDG_STATE_HOME" {
        "LOCALAPPDATA"
    } else {
        "APPDATA"
    };
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .map(|base| base.join("zzy"))
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
        Ok(text) => parse_lenient(&text).unwrap_or_else(|err| {
            eprintln!("zzy: ignoring invalid {}: {err}", path.display());
            T::default()
        }),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => T::default(),
        Err(err) => {
            eprintln!("zzy: cannot read {}: {err}", path.display());
            T::default()
        }
    }
}

/// Parse `text`, skipping keys whose value is invalid (e.g. a theme from a
/// newer zzy, or a typo) rather than ignoring the whole file: they keep their
/// default, and the other settings survive.
fn parse_lenient<T: DeserializeOwned>(text: &str) -> Result<T, toml::de::Error> {
    let table: toml::Table = toml::from_str(text)?;
    let valid: toml::Table = table
        .into_iter()
        .filter(|(key, value)| {
            let single = toml::Table::from_iter([(key.clone(), value.clone())]);
            let ok = T::deserialize(single).is_ok();
            if !ok {
                eprintln!("zzy: ignoring invalid setting {key} = {value}");
            }
            ok
        })
        .collect();
    T::deserialize(valid)
}

fn save<T: Serialize>(path: Option<PathBuf>, value: &T) {
    let Some(path) = path else {
        return;
    };
    if let Err(err) = write_atomically(&path, value) {
        eprintln!("zzy: cannot write {}: {err}", path.display());
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
    fn migrates_the_old_zedterm_directory_once() {
        let base = std::env::temp_dir().join(format!("zzy-migrate-{}", std::process::id()));
        let (old, new) = (base.join("zedterm"), base.join("zzy"));
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("settings.toml"), "font-size = 20.0").unwrap();

        migrate_old_dir(&old, &new);
        assert!(!old.exists());
        assert_eq!(
            fs::read_to_string(new.join("settings.toml")).unwrap(),
            "font-size = 20.0"
        );

        // An existing zzy directory is never overwritten.
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("settings.toml"), "font-size = 30.0").unwrap();
        migrate_old_dir(&old, &new);
        assert!(old.exists());
        assert_eq!(
            fs::read_to_string(new.join("settings.toml")).unwrap(),
            "font-size = 20.0"
        );
        fs::remove_dir_all(&base).unwrap();
    }

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
        assert_eq!(loaded.dark_theme, settings.dark_theme);
        assert_eq!(loaded.light_theme, settings.light_theme);
        assert_eq!(loaded.appearance, settings.appearance);
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
    fn an_invalid_value_keeps_the_other_settings() {
        let loaded: AppSettings =
            parse_lenient("font-size = 20.0\ndark-theme = \"no-such-theme\"").unwrap();
        assert_eq!(loaded.font_size, 20.0);
        assert_eq!(loaded.dark_theme, AppSettings::default().dark_theme);
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
