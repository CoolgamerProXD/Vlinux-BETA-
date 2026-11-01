//! VLINUX core. Module layout:
//!
//! * `config`  — validated TOML configuration, kept apart from any distro files
//! * `system`  — read-only host information
//! * `window`  — full-screen / exit-to-Windows / quit
//!
//! Later milestones add `wsl` (detection + management), `pty` (terminal sessions),
//! `fs` (file manager) and `distro` (registry/installer) as separate modules behind
//! a backend trait so that non-WSL execution backends can be added.

mod config;
mod error;
mod system;
mod window;

use std::sync::atomic::AtomicBool;

use tauri::{Manager, State, WindowEvent};

use crate::config::{Config, ConfigStore};
use crate::error::AppError;
use crate::window::FullscreenTracker;

#[derive(serde::Serialize)]
struct ConfigPayload {
    config: Config,
    path: String,
    /// Set when the on-disk file was unreadable/invalid and defaults are active.
    warning: Option<String>,
}

#[tauri::command]
fn get_config(store: State<'_, ConfigStore>) -> Result<ConfigPayload, AppError> {
    let config = store
        .current
        .lock()
        .map_err(|_| AppError::Config("config lock poisoned".into()))?
        .clone();
    let warning = store
        .warning
        .lock()
        .map_err(|_| AppError::Config("config lock poisoned".into()))?
        .clone();
    Ok(ConfigPayload { config, path: store.path.display().to_string(), warning })
}

/// Validate, persist and return the sanitised configuration.
#[tauri::command]
fn save_config(store: State<'_, ConfigStore>, config: Config) -> Result<Config, AppError> {
    let config = config.sanitized();
    config::save(&store.path, &config)?;
    *store
        .current
        .lock()
        .map_err(|_| AppError::Config("config lock poisoned".into()))? = config.clone();
    // A successful save supersedes any earlier load warning.
    if let Ok(mut w) = store.warning.lock() {
        *w = None;
    }
    Ok(config)
}

#[tauri::command]
fn get_system_info() -> system::SystemInfo {
    system::collect()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(FullscreenTracker(AtomicBool::new(false)))
        .setup(|app| {
            let dir = app
                .path()
                .app_config_dir()
                .map_err(|e| format!("cannot resolve config directory: {e}"))?;
            let store = ConfigStore::open(&dir);
            let start_fullscreen = store
                .current
                .lock()
                .map(|c| c.shell.start_fullscreen)
                .unwrap_or(true);
            app.manage(store);

            // The window is created hidden (see tauri.conf.json) so that the user never
            // sees a flash of the wrong state. Always show it, even if a step fails.
            if let Some(win) = app.get_webview_window("main") {
                if start_fullscreen {
                    if let Err(e) = win.set_fullscreen(true) {
                        eprintln!("vlinux: could not enter full-screen: {e}");
                    }
                }
                if let Err(e) = win.show() {
                    eprintln!("vlinux: could not show window: {e}");
                }
                let _ = win.set_focus();
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::Resized(_) = event {
                let tracker = window.state::<FullscreenTracker>();
                window::on_window_resized(window, &tracker);
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            get_system_info,
            window::set_fullscreen,
            window::is_fullscreen,
            window::exit_to_windows,
            window::quit_app,
        ])
        .run(tauri::generate_context!())
        .expect("error while running VLINUX");
}
