//! Process execution abstraction.
//!
//! Every external program VLINUX runs goes through [`CommandRunner`]. Properties:
//! * arguments are passed as an argv vector — **never through a shell**;
//! * every call has a timeout and can be cancelled;
//! * on Windows no console window flashes (`CREATE_NO_WINDOW`);
//! * tests substitute [`crate::testing::FakeRunner`] so CI needs no real WSL.

use std::fmt;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub timeout: Duration,
    pub env: Vec<(String, String)>,
}

impl CommandSpec {
    pub fn new(program: impl Into<String>, args: &[&str], timeout: Duration) -> Self {
        Self {
            program: program.into(),
            args: args.iter().map(|a| a.to_string()).collect(),
            timeout,
            env: Vec::new(),
        }
    }

    pub fn with_env(mut self, key: &str, value: &str) -> Self {
        self.env.push((key.to_string(), value.to_string()));
        self
    }

    /// `program arg1 arg2` for logs/tests (not for execution).
    pub fn display(&self) -> String {
        let mut s = self.program.clone();
        for a in &self.args {
            s.push(' ');
            s.push_str(a);
        }
        s
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CmdOutput {
    /// `None` if the process was terminated by a signal / has no exit code.
    pub status: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl CmdOutput {
    pub fn success(&self) -> bool {
        self.status == Some(0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
    /// The program does not exist / could not be located.
    NotFound(String),
    Timeout(Duration),
    Cancelled,
    Io(String),
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunError::NotFound(p) => write!(f, "{p} was not found"),
            RunError::Timeout(d) => {
                write!(f, "the command did not finish within {} s", d.as_secs())
            }
            RunError::Cancelled => write!(f, "the operation was cancelled"),
            RunError::Io(m) => write!(f, "could not run the command: {m}"),
        }
    }
}

impl std::error::Error for RunError {}

/// Cooperative cancellation flag shared between the UI thread and a worker.
#[derive(Clone, Default, Debug)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

pub trait CommandRunner: Send + Sync {
    /// Run to completion. `on_output` receives raw stdout/stderr chunks as they arrive.
    fn run(
        &self,
        spec: &CommandSpec,
        cancel: &CancelToken,
        on_output: &mut dyn FnMut(&[u8]),
    ) -> Result<CmdOutput, RunError>;

    /// Convenience: no cancellation, ignore streaming output.
    fn run_simple(&self, spec: &CommandSpec) -> Result<CmdOutput, RunError> {
        self.run(spec, &CancelToken::new(), &mut |_| {})
    }
}

/// Real implementation backed by `std::process`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemRunner;

fn pump<R: Read + Send + 'static>(
    mut reader: R,
    is_err: bool,
    tx: Sender<(bool, Vec<u8>)>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send((is_err, buf[..n].to_vec())).is_err() {
                        break;
                    }
                }
            }
        }
    })
}

const POLL: Duration = Duration::from_millis(40);
const DRAIN_GRACE: Duration = Duration::from_millis(250);

impl CommandRunner for SystemRunner {
    fn run(
        &self,
        spec: &CommandSpec,
        cancel: &CancelToken,
        on_output: &mut dyn FnMut(&[u8]),
    ) -> Result<CmdOutput, RunError> {
        let mut cmd = Command::new(&spec.program);
        cmd.args(&spec.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }

        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                RunError::NotFound(spec.program.clone())
            } else {
                RunError::Io(e.to_string())
            }
        })?;

        let (tx, rx) = mpsc::channel::<(bool, Vec<u8>)>();
        if let Some(out) = child.stdout.take() {
            pump(out, false, tx.clone());
        }
        if let Some(err) = child.stderr.take() {
            pump(err, true, tx.clone());
        }
        drop(tx);

        let mut out = CmdOutput::default();
        let started = Instant::now();
        let mut readers_done = false;

        let status = loop {
            if cancel.is_cancelled() {
                let _ = child.kill();
                let _ = child.wait();
                return Err(RunError::Cancelled);
            }
            if started.elapsed() >= spec.timeout {
                let _ = child.kill();
                let _ = child.wait();
                return Err(RunError::Timeout(spec.timeout));
            }
            if readers_done {
                thread::sleep(POLL);
            } else {
                match rx.recv_timeout(POLL) {
                    Ok((is_err, chunk)) => {
                        on_output(&chunk);
                        if is_err {
                            out.stderr.extend_from_slice(&chunk);
                        } else {
                            out.stdout.extend_from_slice(&chunk);
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => readers_done = true,
                }
            }
            match child.try_wait() {
                Ok(Some(st)) => break st,
                Ok(None) => {}
                Err(e) => {
                    let _ = child.kill();
                    return Err(RunError::Io(e.to_string()));
                }
            }
        };

        // The process has exited: collect whatever the reader threads still deliver, but never
        // wait indefinitely (a grandchild could keep the pipe open).
        let drain_until = Instant::now() + DRAIN_GRACE;
        if !readers_done {
            loop {
                let left = drain_until.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                match rx.recv_timeout(left) {
                    Ok((is_err, chunk)) => {
                        on_output(&chunk);
                        if is_err {
                            out.stderr.extend_from_slice(&chunk);
                        } else {
                            out.stdout.extend_from_slice(&chunk);
                        }
                    }
                    Err(_) => break,
                }
            }
        }

        out.status = status.code();
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_program_is_not_found() {
        let spec = CommandSpec::new(
            "vlinux-definitely-not-a-program",
            &[],
            Duration::from_secs(5),
        );
        assert!(matches!(
            SystemRunner.run_simple(&spec),
            Err(RunError::NotFound(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn captures_output_and_exit_code() {
        let spec = CommandSpec::new(
            "sh",
            &["-c", "printf hello; printf oops >&2; exit 3"],
            Duration::from_secs(10),
        );
        let out = SystemRunner.run_simple(&spec).unwrap();
        assert_eq!(out.status, Some(3));
        assert_eq!(out.stdout, b"hello");
        assert_eq!(out.stderr, b"oops");
    }

    #[cfg(unix)]
    #[test]
    fn times_out_and_kills() {
        let spec = CommandSpec::new("sleep", &["30"], Duration::from_millis(300));
        let t = Instant::now();
        assert_eq!(
            SystemRunner.run_simple(&spec),
            Err(RunError::Timeout(Duration::from_millis(300)))
        );
        assert!(t.elapsed() < Duration::from_secs(5));
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_stops_the_process() {
        let cancel = CancelToken::new();
        let c2 = cancel.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            c2.cancel();
        });
        let spec = CommandSpec::new("sleep", &["30"], Duration::from_secs(30));
        let t = Instant::now();
        assert_eq!(
            SystemRunner.run(&spec, &cancel, &mut |_| {}),
            Err(RunError::Cancelled)
        );
        assert!(t.elapsed() < Duration::from_secs(5));
    }

    #[cfg(unix)]
    #[test]
    fn arguments_are_not_interpreted_by_a_shell() {
        let spec = CommandSpec::new("echo", &["$HOME; rm -rf /"], Duration::from_secs(5));
        let out = SystemRunner.run_simple(&spec).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim(),
            "$HOME; rm -rf /"
        );
    }

    #[test]
    fn spec_display() {
        let s = CommandSpec::new("wsl.exe", &["--status"], Duration::from_secs(1));
        assert_eq!(s.display(), "wsl.exe --status");
    }
}
