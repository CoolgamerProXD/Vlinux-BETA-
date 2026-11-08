//! VLINUX user configuration.
//!
//! * Stored as TOML in the per-user app config directory
//!   (`%APPDATA%\app.vlinux.desktop\config.toml` on Windows).
//! * Deliberately lives *outside* any Linux distribution's files: removing,
//!   resetting or reinstalling a distro can never touch it, and uninstalling
//!   VLINUX never touches distros.
//! * Every value is validated: enums reject unknown strings, numbers are clamped.
//! * An unparseable file is never deleted: it is renamed to `config.toml.invalid-<unix-ts>`
//!   and defaults are used.
//! * A file that parses but has a bad value in one section (for example an unknown theme name)
//!   keeps every other section: only the broken section falls back to its defaults, and the
//!   original file is copied to `config.toml.invalid-<unix-ts>` before it can be overwritten.
//! * Writes are atomic (temp file + rename).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use vlinux_runtime::parse::valid_distro_name;
use vlinux_runtime::storage::StorageConfig;

use crate::error::AppError;

pub const CONFIG_FILE: &str = "config.toml";
/// 3: adds appearance theme/blur, dock labels/magnify/right edge, `[topbar]`, `[terminal]` and
/// `[startup]` (additive; version-1 and version-2 files load unchanged).
/// 2: adds `[runtime]` and `[storage]`.
pub const SCHEMA_VERSION: u32 = 3;

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
    Right,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    Midnight,
    Graphite,
    Oled,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CursorStyle {
    #[default]
    Block,
    Bar,
    Underline,
}

/// What opens by itself when VLINUX starts. Never the terminal: no Linux process starts at launch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StartupApp {
    #[default]
    None,
    Distros,
    Files,
    Settings,
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
    pub theme: Theme,
    /// Backdrop blur behind panels. Turning it off is the biggest GPU saving on integrated graphics.
    pub blur: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DockConfig {
    pub position: DockPosition,
    /// Icon size in CSS pixels (32..=72).
    pub icon_size: u8,
    /// Show app names under the dock icons.
    pub labels: bool,
    /// Enlarge the icon under the pointer.
    pub magnify: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TopbarConfig {
    pub show_workspaces: bool,
    pub show_title: bool,
    pub show_windows: bool,
    pub show_battery: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalConfig {
    /// Font size in CSS pixels (10..=24).
    pub font_size: u8,
    pub cursor: CursorStyle,
    pub cursor_blink: bool,
    /// Lines kept in the scrollback buffer (500..=50000).
    pub scrollback: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StartupConfig {
    pub open_app: StartupApp,
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
    pub topbar: TopbarConfig,
    pub terminal: TerminalConfig,
    pub startup: StartupConfig,
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
            theme: Theme::Midnight,
            blur: true,
        }
    }
}

impl Default for DockConfig {
    fn default() -> Self {
        Self {
            position: DockPosition::Bottom,
            icon_size: 48,
            labels: false,
            magnify: true,
        }
    }
}

impl Default for TopbarConfig {
    fn default() -> Self {
        Self {
            show_workspaces: true,
            show_title: true,
            show_windows: true,
            show_battery: true,
        }
    }
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            font_size: 13,
            cursor: CursorStyle::Block,
            cursor_blink: true,
            scrollback: 5000,
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
            topbar: TopbarConfig::default(),
            terminal: TerminalConfig::default(),
            startup: StartupConfig::default(),
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
        self.terminal.font_size = self.terminal.font_size.clamp(10, 24);
        self.terminal.scrollback = self.terminal.scrollback.clamp(500, 50_000);
        self.runtime.selected_distro = self
            .runtime
            .selected_distro
            .take()
            .filter(|n| valid_distro_name(n));
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
            return Loaded {
                config: Config::default(),
                warning: None,
            };
        }
        Err(e) => {
            return Loaded {
                config: Config::default(),
                warning: Some(format!(
                    "Could not read configuration ({e}); using defaults."
                )),
            };
        }
    };

    // Anything that is not even valid TOML cannot be partly used: keep it and use defaults.
    let table = match text.parse::<toml::Table>() {
        Ok(t) => t,
        Err(parse_err) => return unusable(path, &parse_err.to_string()),
    };
    // Fast path: the whole file is valid.
    if let Ok(cfg) = toml::from_str::<Config>(&text) {
        return Loaded {
            config: cfg.sanitized(),
            warning: None,
        };
    }
    // Something in the file is not valid for this version. Read each section on its own so that
    // one bad value cannot reset the user's other settings.
    let mut cfg = Config::default();
    let mut bad: Vec<String> = Vec::new();
    if let Some(v) = table.get("schema_version") {
        if v.as_integer().is_none() {
            bad.push("schema_version".into());
        }
    }
    section(&table, "shell", &mut cfg.shell, &mut bad);
    section(&table, "appearance", &mut cfg.appearance, &mut bad);
    section(&table, "dock", &mut cfg.dock, &mut bad);
    section(&table, "topbar", &mut cfg.topbar, &mut bad);
    section(&table, "terminal", &mut cfg.terminal, &mut bad);
    section(&table, "startup", &mut cfg.startup, &mut bad);
    section(&table, "runtime", &mut cfg.runtime, &mut bad);
    section(&table, "storage", &mut cfg.storage, &mut bad);
    let list = bad.join(", ");
    let note = match copy_invalid(path) {
        Ok(p) => format!(
            "Some saved settings were not valid ({list}) and were reset to defaults. Everything else was kept. The original file was copied to {}.",
            p.display()
        ),
        Err(e) => format!(
            "Some saved settings were not valid ({list}) and were reset to defaults; everything else was kept. The original could not be copied aside ({e})."
        ),
    };
    Loaded {
        config: cfg.sanitized(),
        warning: Some(note),
    }
}

/// Read one `[name]` table into `slot`; on failure leave the default and record the name.
fn section<T: serde::de::DeserializeOwned>(
    table: &toml::Table,
    name: &str,
    slot: &mut T,
    bad: &mut Vec<String>,
) {
    let Some(v) = table.get(name) else {
        return;
    };
    match v.clone().try_into::<T>() {
        Ok(parsed) => *slot = parsed,
        Err(_) => bad.push(format!("[{name}]")),
    }
}

fn unusable(path: &Path, why: &str) -> Loaded {
    let note = match preserve_invalid(path) {
        Ok(p) => format!(
            "Your configuration file was invalid ({}). It was kept as {} and defaults are in use.",
            first_line(why),
            p.display()
        ),
        Err(e) => format!(
            "Your configuration file was invalid ({}) and could not be backed up ({e}); defaults are in use and the file was left untouched.",
            first_line(why)
        ),
    };
    Loaded {
        config: Config::default(),
        warning: Some(note),
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").to_string()
}

fn invalid_name(path: &Path) -> PathBuf {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let name = format!(
        "{}.invalid-{ts}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(CONFIG_FILE)
    );
    path.with_file_name(name)
}

/// Rename a broken config out of the way (never delete it).
fn preserve_invalid(path: &Path) -> std::io::Result<PathBuf> {
    let dest = invalid_name(path);
    fs::rename(path, &dest)?;
    Ok(dest)
}

/// Copy a partly invalid config aside; the file itself stays, so nothing is lost when it is saved over.
fn copy_invalid(path: &Path) -> std::io::Result<PathBuf> {
    let dest = invalid_name(path);
    fs::copy(path, &dest)?;
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
            if let (Some(toml::Value::Table(new_table)), toml::Value::Table(old_table)) =
                (fresh.get_mut(key), old)
            {
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
    let Some(old) = fs::read_to_string(path)
        .ok()
        .and_then(|t| t.parse::<toml::Table>().ok())
    else {
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
        Err(e) => Err(AppError::Config(format!(
            "could not keep the unknown settings in {} ({e}); nothing was written",
            path.display()
        ))),
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
        fs::write(
            &path,
            "schema_version = 1\n[appearance]\naccent = \"cyan\"\n",
        )
        .unwrap();
        let l = load(&path);
        assert!(l.warning.is_none());
        assert_eq!(l.config.appearance.accent, Accent::Cyan);
        assert_eq!(l.config.storage, StorageConfig::default());
        assert_eq!(l.config.runtime, RuntimeConfig::default());
        assert_eq!(l.config.schema_version, 3);
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
        assert!(!fs::read_to_string(&path)
            .unwrap()
            .contains("selected_distro"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn unsafe_selected_distro_names_are_dropped() {
        let mut c = Config::default();
        c.runtime.selected_distro = Some("x; calc".into());
        assert_eq!(c.sanitized().runtime.selected_distro, None);
        let mut c = Config::default();
        c.runtime.selected_distro = Some("Ubuntu-24.04".into());
        assert_eq!(
            c.sanitized().runtime.selected_distro.as_deref(),
            Some("Ubuntu-24.04")
        );
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
        assert!(
            table
                .get("runtime")
                .and_then(|r| r.get("selected_distro"))
                .is_none(),
            "{text}"
        );
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

    #[test]
    fn one_bad_section_does_not_reset_the_other_settings() {
        let dir = temp_dir("lenient");
        let path = dir.join(CONFIG_FILE);
        let original = "schema_version = 3\n[shell]\nworkspace_count = 7\n[appearance]\naccent = \"mint\"\ntheme = \"neon-pink\"\n[dock]\nposition = \"right\"\nlabels = true\n[terminal]\nfont_size = 16\n";
        fs::write(&path, original).unwrap();
        let l = load(&path);
        // The broken [appearance] is reset; the rest survives.
        assert_eq!(l.config.appearance, AppearanceConfig::default());
        assert_eq!(l.config.shell.workspace_count, 7);
        assert_eq!(l.config.dock.position, DockPosition::Right);
        assert!(l.config.dock.labels);
        assert_eq!(l.config.terminal.font_size, 16);
        let w = l.warning.expect("the user is told");
        assert!(w.contains("[appearance]") && !w.contains("[dock]"), "{w}");
        // The file itself is untouched and a copy was kept.
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        let copies: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".invalid-"))
            .collect();
        assert_eq!(copies.len(), 1);
        assert_eq!(fs::read_to_string(copies[0].path()).unwrap(), original);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn an_unparseable_file_is_renamed_and_defaults_are_used() {
        let dir = temp_dir("garbage");
        let path = dir.join(CONFIG_FILE);
        fs::write(&path, "[shell\nthis is = = not toml").unwrap();
        let l = load(&path);
        assert_eq!(l.config, Config::default());
        assert!(l.warning.is_some());
        assert!(!path.exists(), "moved aside");
        assert_eq!(
            fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().contains(".invalid-")).count(),
            1
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn version_2_files_gain_the_new_settings_with_defaults_and_keep_their_values() {
        let dir = temp_dir("v2");
        let path = dir.join(CONFIG_FILE);
        fs::write(
            &path,
            "schema_version = 2\n[appearance]\nwallpaper = \"peaks\"\nglass = 20\n[dock]\nposition = \"left\"\nicon_size = 40\n[runtime]\nselected_distro = \"archlinux\"\n",
        )
        .unwrap();
        let l = load(&path);
        assert!(l.warning.is_none(), "{:?}", l.warning);
        let c = l.config;
        assert_eq!(c.appearance.wallpaper, Wallpaper::Peaks);
        assert_eq!(c.appearance.glass, 20);
        assert_eq!(c.appearance.theme, Theme::Midnight);
        assert!(c.appearance.blur);
        assert_eq!(c.dock.position, DockPosition::Left);
        assert!(c.dock.magnify && !c.dock.labels);
        assert_eq!(c.topbar, TopbarConfig::default());
        assert_eq!(c.terminal, TerminalConfig::default());
        assert_eq!(c.startup.open_app, StartupApp::None);
        assert_eq!(c.runtime.selected_distro.as_deref(), Some("archlinux"));
        // Loading never rewrites the file.
        assert!(fs::read_to_string(&path).unwrap().contains("schema_version = 2"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn terminal_and_new_appearance_values_are_validated_and_roundtrip() {
        let dir = temp_dir("m7");
        let path = dir.join(CONFIG_FILE);
        let mut cfg = Config::default();
        cfg.appearance.theme = Theme::Oled;
        cfg.appearance.blur = false;
        cfg.topbar.show_title = false;
        cfg.terminal.cursor = CursorStyle::Bar;
        cfg.terminal.font_size = 18;
        cfg.terminal.scrollback = 20_000;
        cfg.startup.open_app = StartupApp::Files;
        save(&path, &cfg).unwrap();
        let l = load(&path);
        assert!(l.warning.is_none());
        assert_eq!(l.config, cfg);
        let mut wild = Config::default();
        wild.terminal.font_size = 200;
        wild.terminal.scrollback = 5;
        let wild = wild.sanitized();
        assert_eq!(wild.terminal.font_size, 24);
        assert_eq!(wild.terminal.scrollback, 500);
        // The terminal can never be a start-up app (no Linux process starts at launch).
        assert!(toml::from_str::<Config>("[startup]\nopen_app = \"terminal\"\n").is_err());
        let _ = fs::remove_dir_all(dir);
    }
}
