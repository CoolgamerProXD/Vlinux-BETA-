//! Pseudoterminal abstraction.
//!
//! The session manager only knows these traits, so everything above them is testable with a fake
//! (see `terminal::testing`). The native implementation uses `portable-pty` (the PTY layer of
//! WezTerm): ConPTY on Windows 10 1809+ / 11, `openpty` on Unix. No shell is involved in starting
//! a process: the program and argument list are passed as-is.

use std::fmt;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PtySize {
    pub rows: u16,
    pub cols: u16,
}

impl PtySize {
    pub const DEFAULT: PtySize = PtySize { rows: 30, cols: 100 };
    pub const MAX_DIMENSION: u32 = 1000;

    /// `None` for zero or absurd sizes.
    pub fn checked(rows: u32, cols: u32) -> Option<PtySize> {
        if rows == 0 || cols == 0 || rows > Self::MAX_DIMENSION || cols > Self::MAX_DIMENSION {
            None
        } else {
            Some(PtySize { rows: rows as u16, cols: cols as u16 })
        }
    }
}

/// What to start. `args` are separate arguments (never a command line to be parsed by a shell).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtySpawn {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: Option<PathBuf>,
    pub size: PtySize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtyError(pub String);

impl fmt::Display for PtyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for PtyError {}

/// Control side of a running PTY.
pub trait PtyControl: Send + Sync {
    fn resize(&self, size: PtySize) -> io::Result<()>;
    /// Ask the child to end (the process VLINUX started — e.g. `wsl.exe` — and nothing else).
    fn kill(&self) -> io::Result<()>;
    /// Release the PTY itself. Idempotent. On Windows this is what makes the output pipe end
    /// after the child has exited.
    fn close(&self);
}

/// Waits for the child. Owned by exactly one thread.
pub trait PtyChild: Send {
    /// Blocks until the child exits; returns its exit code.
    fn wait(&mut self) -> io::Result<u32>;
}

pub struct PtyHandles {
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn Write + Send>,
    pub control: Arc<dyn PtyControl>,
    pub child: Box<dyn PtyChild>,
}

pub trait PtyBackend: Send + Sync {
    fn spawn(&self, spec: &PtySpawn) -> Result<PtyHandles, PtyError>;
}

// ---- native implementation ---------------------------------------------------------------

/// The real pseudoterminal.
pub struct NativePty;

struct NativeControl {
    master: Mutex<Option<Box<dyn portable_pty::MasterPty + Send>>>,
    killer: Mutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl PtyControl for NativeControl {
    fn resize(&self, size: PtySize) -> io::Result<()> {
        match lock(&self.master).as_ref() {
            Some(m) => m
                .resize(portable_pty::PtySize { rows: size.rows, cols: size.cols, pixel_width: 0, pixel_height: 0 })
                .map_err(|e| io::Error::other(format!("{e:#}"))),
            None => Err(io::Error::new(io::ErrorKind::BrokenPipe, "the terminal is closed")),
        }
    }

    fn kill(&self) -> io::Result<()> {
        lock(&self.killer).kill()
    }

    fn close(&self) {
        lock(&self.master).take();
    }
}

struct NativeChild(Box<dyn portable_pty::Child + Send + Sync>);

impl PtyChild for NativeChild {
    fn wait(&mut self) -> io::Result<u32> {
        self.0.wait().map(|s| s.exit_code())
    }
}

impl PtyBackend for NativePty {
    fn spawn(&self, spec: &PtySpawn) -> Result<PtyHandles, PtyError> {
        let err = |what: &str, e: anyhow_like::Error| PtyError(format!("{what}: {e}"));
        let pair = portable_pty::native_pty_system()
            .openpty(portable_pty::PtySize { rows: spec.size.rows, cols: spec.size.cols, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| err("could not create a pseudoterminal", anyhow_like::from(e)))?;

        let mut cmd = portable_pty::CommandBuilder::new(&spec.program);
        for a in &spec.args {
            cmd.arg(a);
        }
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        if let Some(cwd) = &spec.cwd {
            cmd.cwd(cwd);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| err(&format!("could not start '{}'", spec.program), anyhow_like::from(e)))?;
        // The parent must not keep the slave side open, otherwise the reader never sees EOF on Unix.
        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| err("could not open the terminal output", anyhow_like::from(e)))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| err("could not open the terminal input", anyhow_like::from(e)))?;
        let killer = child.clone_killer();

        Ok(PtyHandles {
            reader,
            writer,
            control: Arc::new(NativeControl { master: Mutex::new(Some(pair.master)), killer: Mutex::new(killer) }),
            child: Box::new(NativeChild(child)),
        })
    }
}

/// `portable-pty` reports `anyhow::Error`; this keeps `anyhow` out of our public surface by
/// rendering it (with its cause chain) into a plain string at the boundary.
mod anyhow_like {
    pub struct Error(String);
    impl std::fmt::Display for Error {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.0)
        }
    }
    pub fn from<E: std::fmt::Display + std::fmt::Debug>(e: E) -> Error {
        // `{:#}` prints the whole context chain for anyhow errors.
        Error(format!("{e:#}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_bounded() {
        assert_eq!(PtySize::checked(30, 100), Some(PtySize { rows: 30, cols: 100 }));
        assert_eq!(PtySize::checked(0, 80), None);
        assert_eq!(PtySize::checked(24, 0), None);
        assert_eq!(PtySize::checked(1001, 80), None);
        assert_eq!(PtySize::checked(24, 1000), Some(PtySize { rows: 24, cols: 1000 }));
    }
}
