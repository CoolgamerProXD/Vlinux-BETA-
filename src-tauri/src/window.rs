//! Window / session control: full-screen, exit-to-Windows and quit.
//!
//! Design notes
//! * Full-screen is the regular borderless full-screen of the OS — *not* a kiosk. The
//!   window is never `always_on_top`, so Win key, Alt+Tab, Alt+F4 and the taskbar keep
//!   working. Windows must always remain reachable.
//! * "Exit to Windows" leaves full-screen and minimises. VLINUX (and, in later
//!   milestones, any running Linux distribution) keeps running.
//! * "Quit" ends the VLINUX process. Later milestones hook distro shutdown policy in
//!   `shutdown_hook`; in M0/M1 no Linux processes exist.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Emitter, Runtime, WebviewWindow, Window};

use crate::error::AppError;

pub const FULLSCREEN_EVENT: &str = "vlinux://fullscreen";

/// Tracks the last full-screen state we told the frontend about, so that resize
/// events (which fire continuously) only emit when the state actually changes.
pub struct FullscreenTracker(pub AtomicBool);

#[tauri::command]
pub fn set_fullscreen<R: Runtime>(window: WebviewWindow<R>, fullscreen: bool) -> Result<bool, AppError> {
    if !fullscreen {
        // Make sure we are not stuck minimised/maximised oddly when leaving full-screen.
        let _ = window.unminimize();
    }
    window.set_fullscreen(fullscreen)?;
    Ok(window.is_fullscreen()?)
}

#[tauri::command]
pub fn is_fullscreen<R: Runtime>(window: WebviewWindow<R>) -> Result<bool, AppError> {
    Ok(window.is_fullscreen()?)
}

/// Leave full-screen and minimise to the Windows taskbar. Nothing is terminated.
#[tauri::command]
pub fn exit_to_windows<R: Runtime>(window: WebviewWindow<R>) -> Result<(), AppError> {
    if window.is_fullscreen()? {
        window.set_fullscreen(false)?;
    }
    window.minimize()?;
    Ok(())
}

/// Terminate VLINUX. The frontend asks the user to confirm first.
#[tauri::command]
pub fn quit_app<R: Runtime>(app: AppHandle<R>) {
    shutdown_hook();
    app.exit(0);
}

/// Seam for the user's "stop distro on exit" policy (later milestone). Terminal sessions are
/// ended by the `RunEvent::Exit` handler in `lib.rs`, which covers every way of leaving. VLINUX
/// never shuts WSL or a distribution down on its own.
pub fn shutdown_hook() {}

/// Called from the window-event handler: emit full-screen changes to the frontend.
pub fn on_window_resized<R: Runtime>(window: &Window<R>, tracker: &FullscreenTracker) {
    if let Ok(now) = window.is_fullscreen() {
        if tracker.0.swap(now, Ordering::SeqCst) != now {
            let _ = window.emit(FULLSCREEN_EVENT, now);
        }
    }
}
