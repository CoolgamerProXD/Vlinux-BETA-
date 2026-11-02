//! Tauri bindings for terminal sessions.
//!
//! * The frontend never sends a command line. `terminal_open` names (at most) a distribution; the
//!   backend validates it against the current WSL state and builds the `wsl.exe` arguments itself.
//! * Keystrokes (`terminal_input`) are forwarded to the session's PTY and nothing else.
//! * Output is pushed as events (`vlinux://terminal-output`, base64 bytes) — no polling.
//! * No Linux process is started until the user opens a session.
//! * All process/PTY management lives in `vlinux_runtime::terminal`.

use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use vlinux_runtime::terminal::session::{SessionInfo, SessionState, TerminalManager, TerminalSink};
use vlinux_runtime::terminal::{b64, resolve_target, NativePty, PtySize, TerminalError};
use vlinux_runtime::wsl::Programs;

use crate::config::ConfigStore;
use crate::error::AppError;
use crate::runtime::RuntimeState;

pub const OUTPUT_EVENT: &str = "vlinux://terminal-output";
pub const STATE_EVENT: &str = "vlinux://terminal-state";

#[derive(Serialize, Clone)]
struct OutputPayload {
    id: String,
    /// Raw terminal bytes, base64. They may end mid-character, so they are not sent as text.
    data: String,
}

#[derive(Serialize, Clone)]
struct StatePayload {
    id: String,
    state: SessionState,
}

struct TauriSink {
    app: AppHandle,
}

impl TerminalSink for TauriSink {
    fn output(&self, id: &str, data: &[u8]) {
        let _ = self.app.emit(OUTPUT_EVENT, OutputPayload { id: id.to_string(), data: b64::encode(data) });
    }
    fn state(&self, id: &str, state: &SessionState) {
        let _ = self.app.emit(STATE_EVENT, StatePayload { id: id.to_string(), state: state.clone() });
    }
}

pub struct TerminalState {
    pub manager: TerminalManager,
    programs: Programs,
}

impl TerminalState {
    /// Only constructs the state; no process is started.
    pub fn new(app: AppHandle) -> Self {
        TerminalState {
            manager: TerminalManager::new(Arc::new(NativePty), Arc::new(TauriSink { app })),
            programs: Programs::system(),
        }
    }
}

impl From<TerminalError> for AppError {
    fn from(e: TerminalError) -> Self {
        AppError::Runtime(e.to_string())
    }
}

type Term<'a> = State<'a, Arc<TerminalState>>;

/// Open a terminal session in `distro`, or in the distribution selected in VLINUX when `distro`
/// is `None`. Nothing is installed, and WSL's default distribution is never changed.
#[tauri::command]
pub async fn terminal_open(
    rt: State<'_, Arc<RuntimeState>>,
    term: Term<'_>,
    store: State<'_, ConfigStore>,
    distro: Option<String>,
    rows: u32,
    cols: u32,
) -> Result<SessionInfo, AppError> {
    let rt = rt.inner().clone();
    let term = term.inner().clone();
    let selected = store.current.lock().ok().and_then(|c| c.runtime.selected_distro.clone());
    let size = PtySize::checked(rows, cols).unwrap_or(PtySize::DEFAULT);
    tauri::async_runtime::spawn_blocking(move || -> Result<SessionInfo, AppError> {
        // Validate against the current WSL state. A cached report is fine; if it does not know
        // the distribution (or says WSL is unusable) look again once before refusing.
        let requested = distro.as_deref();
        let mut report = rt.detect(false);
        let mut target = resolve_target(&report, requested, selected.as_deref());
        if matches!(
            target,
            Err(TerminalError::DistroNotFound { .. }) | Err(TerminalError::NoDistributions) | Err(TerminalError::WslUnavailable { .. })
        ) {
            report = rt.detect(true);
            target = resolve_target(&report, requested, selected.as_deref());
        }
        let target = target?;
        Ok(term.manager.open_target(&target, &term.programs, size)?)
    })
    .await
    .map_err(|e| AppError::Runtime(format!("background task failed: {e}")))?
}

/// Forward keystrokes / pasted text to the session's PTY.
#[tauri::command]
pub fn terminal_input(term: Term<'_>, id: String, data: String) -> Result<(), AppError> {
    Ok(term.manager.write(&id, data.as_bytes())?)
}

#[tauri::command]
pub async fn terminal_resize(term: Term<'_>, id: String, rows: u32, cols: u32) -> Result<(), AppError> {
    let term = term.inner().clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), AppError> { Ok(term.manager.resize(&id, rows, cols)?) })
        .await
        .map_err(|e| AppError::Runtime(format!("background task failed: {e}")))?
}

/// The UI has rendered `bytes` of output (flow control).
#[tauri::command]
pub fn terminal_ack(term: Term<'_>, id: String, bytes: u32) {
    term.manager.ack(&id, bytes as usize);
}

/// Close one session. Other sessions, WSL and the distribution keep running.
#[tauri::command]
pub async fn terminal_close(term: Term<'_>, id: String) -> Result<(), AppError> {
    let term = term.inner().clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), AppError> { Ok(term.manager.close(&id)?) })
        .await
        .map_err(|e| AppError::Runtime(format!("background task failed: {e}")))?
}

#[tauri::command]
pub fn terminal_list(term: Term<'_>) -> Vec<SessionInfo> {
    term.manager.list()
}
