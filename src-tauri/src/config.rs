//! VLINUX user configuration.
//!
//! * Stored as TOML in the per-user app config directory
//!   (`%APPDATA%\app.vlinux.desktop\config.toml` on Windows).
//! * Deliberately lives *outside* any Linux distribution's files: removing,
//!   resetting or reinstalling a distro can never touch it, and uninstalling
//!   VLINUX never touches distros.
//! * Every value is validated: enums reject unknown strings, numbers are clamped.
//! * A corrupt file is never deleted: it is renamed to `config.toml.invalid-<unix-ts>`
//!   and defaults are used.
//! * Writes are atomic (temp file + rename).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::AppError;

pub const CONFIG_FILE: &str = "config.toml";
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Wallpaper {
    Nebula,
    Dusk,
    Peaks,
    Grid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Accent {
    Violet,
    Cyan,
    Rose,
    Amber,
    Mint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DockPosition {
    Bottom,
    Left,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShellConfig {
    /// Enter full-screen mode when VLINUX starts.
    pub start_fullscreen: bool,
    pub clock_24h: bool,
    pub show_seconds: bool,
    pub show_date: bool,
    /// Number of workspaces (2..=9).
    pub workspace_count: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppearanceConfig {
    pub wallpaper: Wallpaper,
    pub accent: Accent,
    /// Panel translucency, 0 (opaque) ..= 100 (very glassy).
    pub glass: u8,
    pub animations: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DockConfig {
    pub position: DockPosition,
    /// Icon size in CSS pixels (32..=72).
    pub icon_size: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub schema_version: u32,
    pub shell: ShellConfig,
    pub appearance: AppearanceConfig,
    pub dock: DockConfig,
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            start_fullscreen: true,
            clock_24h: true,
            show_seconds: false,
            show_date: true,
            workspace_count: 5,
        }
    }
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            wallpaper: Wallpaper::Nebula,
            accent: Accent::Violet,
            glass: 60,
            animations: true,
        }
    }
}

impl Default for DockConfig {
    fn default() -> Self {
        Self {
            position: DockPosition::Bottom,
            icon_size: 48,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            shell: ShellConfig::default(),
            appearance: AppearanceConfig::default(),
            dock: DockConfig::default(),
        }
    }
}

impl Config {
    /// Clamp numeric fields into their valid ranges.
    pub fn sanitized(mut self) -> Self {
        self.schema_version = SCHEMA_VERSION;
        self.shell.workspace_count = self.shell.workspace_count.clamp(2, 9);
        self.appearance.glass = self.appearance.glass.min(100);
        self.dock.icon_size = self.dock.icon_size.clamp(32, 72);
        self
    }
}

/// Result of loading the configuration from disk.
pub struct Loaded {
    pub config: Config,
    /// Human-readable note if something went wrong and defaults were used.
    pub warning: Option<String>,
}

pub fn load(path: &Path) -> Loaded {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Loaded { config: Config::default(), warning: None };
        }
        Err(e) => {
            return Loaded {
                config: Config::default(),
                warning: Some(format!("Could not read configuration ({e}); using defaults.")),
            };
        }
    };

    match toml::from_str::<Config>(&text) {
        Ok(cfg) => Loaded { config: cfg.sanitized(), warning: None },
        Err(parse_err) => {
            let note = match preserve_invalid(path) {
                Ok(p) => format!(
                    "Your configuration file was invalid ({}). It was kept as {} and defaults are in use.",
                    first_line(&parse_err.to_string()),
                    p.display()
                ),
                Err(e) => format!(
                    "Your configuration file was invalid ({}) and could not be backed up ({e}); defaults are in use and the file was left untouched.",
                    first_line(&parse_err.to_string())
                ),
            };
            Loaded { config: Config::default(), warning: Some(note) }
        }
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").to_string()
}

/// Rename a broken config out of the way (never delete it).
fn preserve_invalid(path: &Path) -> std::io::Result<PathBuf> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let name = format!(
        "{}.invalid-{ts}",
        path.file_name().and_then(|n| n.to_str()).unwrap_or(CONFIG_FILE)
    );
    let dest = path.with_file_name(name);
    fs::rename(path, &dest)?;
    Ok(dest)
}

/// Atomically write the config (temp file in the same directory, then rename).
pub fn save(path: &Path, cfg: &Config) -> Result<(), AppError> {
    let text = toml::to_string_pretty(cfg).map_err(|e| AppError::Config(e.to_string()))?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, text.as_bytes())?;
    // `rename` replaces the destination on both Unix and Windows (Rust uses MoveFileExW with REPLACE_EXISTING).
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Shared, managed application state.
pub struct ConfigStore {
    pub path: PathBuf,
    pub current: Mutex<Config>,
    pub warning: Mutex<Option<String>>,
}

impl ConfigStore {
    pub fn open(dir: &Path) -> Self {
        let path = dir.join(CONFIG_FILE);
        let loaded = load(&path);
        Self {
            path,
            current: Mutex::new(loaded.config),
            warning: Mutex::new(loaded.warning),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "vlinux-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn missing_file_gives_defaults_without_warning() {
        let dir = temp_dir("missing");
        let l = load(&dir.join(CONFIG_FILE));
        assert_eq!(l.config, Config::default());
        assert!(l.warning.is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn roundtrip_preserves_values() {
        let dir = temp_dir("roundtrip");
        let path = dir.join(CONFIG_FILE);
        let mut cfg = Config::default();
        cfg.appearance.wallpaper = Wallpaper::Peaks;
        cfg.appearance.accent = Accent::Mint;
        cfg.dock.position = DockPosition::Left;
        cfg.shell.workspace_count = 7;
        save(&path, &cfg).unwrap();
        let l = load(&path);
        assert_eq!(l.config, cfg);
        assert!(l.warning.is_none());
        assert!(!path.with_extension("toml.tmp").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn partial_file_fills_defaults() {
        let dir = temp_dir("partial");
        let path = dir.join(CONFIG_FILE);
        fs::write(&path, "[appearance]\naccent = \"rose\"\n").unwrap();
        let l = load(&path);
        assert_eq!(l.config.appearance.accent, Accent::Rose);
        assert_eq!(l.config.shell, ShellConfig::default());
        assert!(l.warning.is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn out_of_range_numbers_are_clamped() {
        let dir = temp_dir("clamp");
        let path = dir.join(CONFIG_FILE);
        fs::write(
            &path,
            "[shell]\nworkspace_count = 200\n[appearance]\nglass = 250\n[dock]\nicon_size = 1\n",
        )
        .unwrap();
        let c = load(&path).config;
        assert_eq!(c.shell.workspace_count, 9);
        assert_eq!(c.appearance.glass, 100);
        assert_eq!(c.dock.icon_size, 32);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn invalid_file_is_preserved_not_deleted() {
        let dir = temp_dir("invalid");
        let path = dir.join(CONFIG_FILE);
        fs::write(&path, "[appearance]\nwallpaper = \"not-a-wallpaper\"\n").unwrap();
        let l = load(&path);
        assert_eq!(l.config, Config::default());
        assert!(l.warning.is_some());
        let backups: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".invalid-"))
            .collect();
        assert_eq!(backups.len(), 1, "the broken file must be kept");
        let kept = fs::read_to_string(backups[0].path()).unwrap();
        assert!(kept.contains("not-a-wallpaper"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn json_from_frontend_deserialises() {
        let json = r#"{"schema_version":1,"shell":{"start_fullscreen":false,"clock_24h":false,"show_seconds":true,"show_date":true,"workspace_count":4},
            "appearance":{"wallpaper":"dusk","accent":"amber","glass":30,"animations":false},
            "dock":{"position":"left","icon_size":56}}"#;
        let c: Config = serde_json::from_str(json).unwrap();
        assert_eq!(c.appearance.wallpaper, Wallpaper::Dusk);
        assert_eq!(c.dock.position, DockPosition::Left);
        assert!(serde_json::from_str::<Config>(r#"{"dock":{"position":"top"}}"#).is_err());
    }
}
