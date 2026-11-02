//! Interactive terminal sessions (M3).
//!
//! A session is a real pseudoterminal (ConPTY on Windows) running `wsl.exe -d <distro>`, which
//! starts the distribution's own default login shell. VLINUX never interprets what is typed:
//! keystrokes go to the PTY, output comes back as raw bytes for the terminal emulator in the UI.
//!
//! * [`pty`]     — the PTY abstraction ([`pty::PtyBackend`]) and the native implementation
//! * [`launch`]  — validation of the target distribution and the (fixed) `wsl.exe` command lines
//! * [`session`] — [`session::TerminalManager`]: sessions, threads, flow control, cleanup
//! * [`b64`]     — tiny base64 encoder used to carry bytes over JSON events
//! * [`testing`] — fake PTY and recording sink for tests (no distribution needed)

pub mod b64;
pub mod launch;
pub mod pty;
pub mod session;
pub mod testing;

use std::fmt;

pub use launch::{build_plan, resolve_target, LaunchAttempt, LaunchPlan, Target};
pub use pty::{NativePty, PtyBackend, PtyError, PtySize, PtySpawn};
pub use session::{SessionInfo, SessionState, TerminalManager, TerminalSink, Tuning};

/// Everything that can go wrong when opening or using a terminal session. Messages are written
/// for the user and say what to do next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalError {
    /// WSL itself is not usable (not installed, restart pending, unsupported host, ...).
    WslUnavailable { reason: String },
    /// WSL works but has no distributions.
    NoDistributions,
    /// Nothing is selected and nothing was requested.
    NoSelection,
    /// The name is not a valid WSL distribution name (never reaches `wsl.exe`).
    InvalidDistro { name: String },
    /// A valid name that is not registered with WSL (for example it was removed).
    DistroNotFound { name: String, available: Vec<String> },
    /// A distribution that belongs to another tool (Docker Desktop, ...).
    DistroNotAllowed { name: String },
    /// The distribution is being installed, uninstalled or converted.
    DistroBusy { name: String, state: String },
    TooManySessions { max: usize },
    /// `wsl.exe` (or the PTY) could not be started.
    StartFailed { detail: String },
    UnknownSession { id: String },
    SessionEnded { id: String },
    InvalidSize { rows: u32, cols: u32 },
    InputTooLarge { len: usize, max: usize },
}

impl fmt::Display for TerminalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TerminalError::WslUnavailable { reason } => {
                write!(f, "A Linux terminal needs WSL, which is not ready: {reason} Open Linux Setup to see what is needed.")
            }
            TerminalError::NoDistributions => {
                write!(f, "No Linux distribution is installed yet. Open Linux Setup to choose and install one.")
            }
            TerminalError::NoSelection => {
                write!(f, "No distribution is selected for VLINUX. Pick one in Linux Setup, or choose one from the terminal's new-tab menu.")
            }
            TerminalError::InvalidDistro { name } => {
                write!(f, "'{}' is not a valid distribution name.", name.chars().take(40).collect::<String>())
            }
            TerminalError::DistroNotFound { name, available } => {
                if available.is_empty() {
                    write!(f, "The distribution '{name}' is not installed (it may have been removed). Open Linux Setup to choose another.")
                } else {
                    write!(f, "The distribution '{name}' is not installed (it may have been removed). Installed: {}. Open Linux Setup to choose another.", available.join(", "))
                }
            }
            TerminalError::DistroNotAllowed { name } => {
                write!(f, "'{name}' belongs to another application and is never opened by VLINUX.")
            }
            TerminalError::DistroBusy { name, state } => {
                write!(f, "'{name}' is {state} right now. Try again when WSL has finished.")
            }
            TerminalError::TooManySessions { max } => {
                write!(f, "Too many terminal sessions are open (limit {max}). Close one first.")
            }
            TerminalError::StartFailed { detail } => write!(f, "The terminal could not be started: {detail}"),
            TerminalError::UnknownSession { id } => write!(f, "There is no terminal session '{id}'."),
            TerminalError::SessionEnded { id } => write!(f, "The terminal session '{id}' has ended."),
            TerminalError::InvalidSize { rows, cols } => write!(f, "Invalid terminal size {cols}x{rows}."),
            TerminalError::InputTooLarge { len, max } => write!(f, "Input of {len} bytes is too large (limit {max})."),
        }
    }
}

impl std::error::Error for TerminalError {}
