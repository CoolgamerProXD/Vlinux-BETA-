//! VLINUX core. Module layout:
//!
//! * `config`  — validated TOML configuration, kept apart from any distro files
//! * `system`  — read-only host information
//! * `window`  — full-screen / exit-to-Windows / quit
//! * `runtime` — Tauri bindings for the `vlinux-runtime` crate (WSL detection, install plans,
//!   storage model). All Linux-runtime logic lives in that Tauri-free crate.
//! * `terminal` — Tauri bindings for interactive terminal sessions (PTY logic is in the runtime crate)
//!
//! * `files`  — Tauri bindings for the file manager and shared-folder mappings (logic is in the runtime crate)
//! * `distros` — Tauri bindings for the Distribution Manager: overview, lifecycle, inspection, package
//!   plans, `/etc/wsl.conf` editing and compatibility diagnostics (logic is in the runtime crate)
//!
//! Later milestones add full distro
//! management, behind a backend trait so that non-WSL execution backends can be added.

mod config;
mod distros;
mod error;
mod files;
mod runtime;
mod system;
mod terminal;
mod window;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

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
    let app = tauri::Builder::default()
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
            // Only constructs the state. No wsl.exe is run until the UI asks for detection.
            let rt = Arc::new(runtime::RuntimeState::new(&dir));
            app.manage(rt.clone());
            // The file manager and shared-folder mappings. Nothing runs inside a distribution
            // until the user opens Files or checks/applies a mapping.
            app.manage(Arc::new(files::FilesState::new(app.handle().clone(), rt.clone(), &dir)));
            // Distribution Manager: only constructs state. Nothing is inspected, started or polled
            // until the user asks.
            app.manage(Arc::new(distros::DistroMgmt::new(app.handle().clone(), rt, &dir)));
            // Likewise: no PTY and no Linux process exists until the user opens a terminal.
            app.manage(Arc::new(terminal::TerminalState::new(app.handle().clone())));

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
            runtime::wsl_detect,
            runtime::wsl_catalog,
            runtime::install_prepare,
            runtime::install_start,
            runtime::install_cancel,
            runtime::storage_overview,
            runtime::storage_validate_path,
            terminal::terminal_open,
            terminal::terminal_input,
            terminal::terminal_resize,
            terminal::terminal_ack,
            terminal::terminal_close,
            terminal::terminal_list,
            files::fs_places,
            files::fs_list_open,
            files::fs_list_page,
            files::fs_list_close,
            files::fs_create,
            files::fs_rename,
            files::fs_delete,
            files::fs_transfer,
            files::fs_job_cancel,
            files::fs_open,
            files::fs_reveal,
            files::mapping_status,
            files::mapping_apply,
            files::mapping_undo,
            distros::distro_overview,
            distros::distro_set_preferred,
            distros::distro_start,
            distros::distro_stop,
            distros::distro_inspect,
            distros::distro_inspect_cancel,
            distros::distro_pkg_plan,
            distros::distro_pkg_search,
            distros::wslconf_read,
            distros::wslconf_preview,
            distros::wslconf_apply,
            distros::wslconf_backups,
            distros::wslconf_restore,
            distros::wslconfig_windows,
            distros::diag_run,
            distros::diag_cancel,
        ])
        .build(tauri::generate_context!())
        .expect("error while building VLINUX");

    app.run(|handle, event| {
        // Whatever the exit path (menu, Alt+F4, installer), end the terminal sessions VLINUX
        // started. Only those processes; WSL and the distributions keep running.
        if let tauri::RunEvent::Exit = event {
            if let Some(t) = handle.try_state::<Arc<terminal::TerminalState>>() {
                t.manager.close_all();
            }
        }
    });
}
