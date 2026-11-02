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

use vlinux_runtime::parse::valid_distro_name;
use vlinux_runtime::storage::StorageConfig;

use crate::error::AppError;

pub const CONFIG_FILE: &str = "config.toml";
/// 2: adds `[runtime]` and `[storage]` (additive; version-1 files load unchanged).
pub const SCHEMA_VERSION: u32 = 2;

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

/// Linux runtime preferences. Only stores a *choice*; nothing here starts or changes a distro.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeConfig {
    /// Registered WSL name of the distribution the user chose to use with VLINUX. Never set
    /// automatically, and never written to WSL (VLINUX does not run `wsl --set-default`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_distro: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub schema_version: u32,
    pub shell: ShellConfig,
    pub appearance: AppearanceConfig,
    pub dock: DockConfig,
    pub runtime: RuntimeConfig,
    pub storage: StorageConfig,
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
            runtime: RuntimeConfig::default(),
            storage: StorageConfig::default(),
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
        self.runtime.selected_distro = self.runtime.selected_distro.take().filter(|n| valid_distro_name(n));
        self.storage = self.storage.sanitized();
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

/// Keys that are skipped when `None`, so they legitimately vanish from a freshly serialised config.
/// Everything that is neither written by the current schema nor listed here is *unknown* (for
/// example a setting added by a newer VLINUX) and is carried over on save.
fn is_optional_known(path: &[String]) -> bool {
    match path {
        [a, b] => a == "runtime" && b == "selected_distro",
        [a, _, c] => a == "storage" && c == "windows_path",
        _ => false,
    }
}

/// Copy keys that the current schema does not know from the file on disk into the new document.
/// Known keys always take the new value. Returns how many top-level or nested keys were carried.
fn carry_unknown(existing: &toml::Table, fresh: &mut toml::Table, path: &mut Vec<String>) -> usize {
    let mut carried = 0;
    for (key, old) in existing {
        path.push(key.clone());
        if fresh.contains_key(key) {
            if let (Some(toml::Value::Table(new_table)), toml::Value::Table(old_table)) = (fresh.get_mut(key), old) {
                carried += carry_unknown(old_table, new_table, path);
            }
        } else if !is_optional_known(path) {
            fresh.insert(key.clone(), old.clone());
            carried += 1;
        }
        path.pop();
    }
    carried
}

/// Serialise `cfg`, keeping any settings in the existing file that this version does not know
/// (so saving never discards what a newer or older VLINUX wrote). Comments are not preserved.
fn render(path: &Path, cfg: &Config) -> Result<(String, usize), AppError> {
    let plain = toml::to_string_pretty(cfg).map_err(|e| AppError::Config(e.to_string()))?;
    let Some(old) = fs::read_to_string(path).ok().and_then(|t| t.parse::<toml::Table>().ok()) else {
        return Ok((plain, 0));
    };
    let Ok(toml::Value::Table(mut fresh)) = toml::Value::try_from(cfg) else {
        return Ok((plain, 0));
    };
    let carried = carry_unknown(&old, &mut fresh, &mut Vec::new());
    if carried == 0 {
        return Ok((plain, 0));
    }
    match toml::to_string_pretty(&fresh) {
        Ok(text) => Ok((text, carried)),
        Err(e) => Err(AppError::Config(format!("could not keep the unknown settings in {} ({e}); nothing was written", path.display()))),
    }
}

/// Atomically write the config (temp file in the same directory, then rename). Settings this
/// version does not know are kept; if they cannot be kept the save is refused rather than
/// silently dropping them.
pub fn save(path: &Path, cfg: &Config) -> Result<(), AppError> {
    let (text, _carried) = render(path, cfg)?;
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
    fn version_1_files_load_with_new_sections_defaulted() {
        let dir = temp_dir("v1");
        let path = dir.join(CONFIG_FILE);
        fs::write(&path, "schema_version = 1\n[appearance]\naccent = \"cyan\"\n").unwrap();
        let l = load(&path);
        assert!(l.warning.is_none());
        assert_eq!(l.config.appearance.accent, Accent::Cyan);
        assert_eq!(l.config.storage, StorageConfig::default());
        assert_eq!(l.config.runtime, RuntimeConfig::default());
        assert_eq!(l.config.schema_version, 2);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn runtime_and_storage_preferences_roundtrip_through_toml() {
        use vlinux_runtime::storage::{MappingMode, ProjectsMode};
        let dir = temp_dir("rt2");
        let path = dir.join(CONFIG_FILE);
        let mut cfg = Config::default();
        cfg.runtime.selected_distro = Some("archlinux".into());
        cfg.storage.music.mode = MappingMode::Private;
        cfg.storage.documents.windows_path = Some("D:\\Docs".into());
        cfg.storage.projects = ProjectsMode::Shared;
        save(&path, &cfg).unwrap();
        let l = load(&path);
        assert!(l.warning.is_none(), "{:?}", l.warning);
        assert_eq!(l.config, cfg);
        // The unset selection is simply absent, not an empty string.
        let mut none = Config::default();
        none.runtime.selected_distro = None;
        save(&path, &none).unwrap();
        assert!(!fs::read_to_string(&path).unwrap().contains("selected_distro"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn unsafe_selected_distro_names_are_dropped() {
        let mut c = Config::default();
        c.runtime.selected_distro = Some("x; calc".into());
        assert_eq!(c.sanitized().runtime.selected_distro, None);
        let mut c = Config::default();
        c.runtime.selected_distro = Some("Ubuntu-24.04".into());
        assert_eq!(c.sanitized().runtime.selected_distro.as_deref(), Some("Ubuntu-24.04"));
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

    #[test]
    fn saving_keeps_settings_this_version_does_not_know() {
        let dir = temp_dir("unknown-keys");
        let path = dir.join("config.toml");
        fs::write(
            &path,
            "future_top = 7\nschema_version = 2\n\n[shell]\nworkspace_count = 5\nfuture_shell_flag = true\n\n[future_section]\nname = \"x\"\n\n[runtime]\nselected_distro = \"Old\"\n",
        )
        .unwrap();
        let mut cfg = load(&path).config;
        assert_eq!(cfg.runtime.selected_distro.as_deref(), Some("Old"));
        cfg.shell.workspace_count = 6;
        cfg.runtime.selected_distro = None;
        save(&path, &cfg).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        let table: toml::Table = text.parse().unwrap();
        assert_eq!(table["future_top"].as_integer(), Some(7));
        assert_eq!(table["future_section"]["name"].as_str(), Some("x"));
        assert_eq!(table["shell"]["future_shell_flag"].as_bool(), Some(true));
        assert_eq!(table["shell"]["workspace_count"].as_integer(), Some(6));
        // A known optional key that was cleared really is cleared (it is not resurrected).
        assert!(table.get("runtime").and_then(|r| r.get("selected_distro")).is_none(), "{text}");
        // The result still loads as a normal config.
        assert_eq!(load(&path).config.shell.workspace_count, 6);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn saving_without_unknown_settings_is_unchanged() {
        let dir = temp_dir("no-unknown");
        let path = dir.join("config.toml");
        let cfg = Config::default();
        save(&path, &cfg).unwrap();
        let first = fs::read_to_string(&path).unwrap();
        save(&path, &cfg).unwrap();
        assert_eq!(first, fs::read_to_string(&path).unwrap());
        assert_eq!(first, toml::to_string_pretty(&cfg).unwrap());
        let _ = fs::remove_dir_all(&dir);
    }
}
