//! Terminal sessions: one PTY + one process per session, all process management in Rust.
//!
//! Threads per running attempt (named `vlinux-term-*`):
//! * **reader** — blocks on the PTY output, forwards raw bytes to the [`TerminalSink`]. No polling.
//!   It pauses while too much output is unacknowledged by the UI (flow control), so a command like
//!   `yes` cannot flood the desktop.
//! * **writer** — drains an input queue into the PTY, so forwarding a keystroke never blocks the
//!   caller even if the shell is not reading.
//! * **waiter** — waits for the process, lets the last output drain, releases the PTY, reports the
//!   exit and (only for an immediate startup failure) starts the fallback shell.
//!
//! Closing a session kills only the process VLINUX started for it (`wsl.exe`); WSL, the
//! distribution and other sessions are untouched, and nothing is ever unregistered or reset.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;

use super::launch::{build_plan, LaunchPlan, Target};
use super::pty::{PtyBackend, PtyControl, PtyError, PtySize};
use super::TerminalError;
use crate::unix_now;
use crate::wsl::Programs;

/// Receives everything a session produces. Implemented by the Tauri layer (events) and by tests.
pub trait TerminalSink: Send + Sync {
    /// Raw terminal output. May end in the middle of a UTF-8 sequence or escape sequence.
    fn output(&self, id: &str, data: &[u8]);
    /// The session changed state (it ended, or failed).
    fn state(&self, id: &str, state: &SessionState);
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionState {
    Running,
    /// The shell ended. `code` is the process exit code when known.
    Exited {
        code: Option<i32>,
    },
    /// The session could not be started or its process could not be tracked.
    Failed {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionInfo {
    pub id: String,
    pub distro: String,
    pub label: String,
    pub state: SessionState,
    pub started_at: u64,
    /// Which launch attempt is running: `default shell` or the `/bin/sh` fallback.
    pub shell: String,
}

#[derive(Debug, Clone, Copy)]
pub struct Tuning {
    pub max_sessions: usize,
    /// The reader pauses while this many bytes are unacknowledged.
    pub high_water: usize,
    /// ...but never longer than this (a UI that stopped acknowledging must not hang a session).
    pub stall_timeout: Duration,
    /// After the process exits, how long the output must be quiet before the PTY is released.
    pub drain_quiet: Duration,
    /// A non-zero exit within this time of starting counts as "could not start the shell".
    pub fast_fail: Duration,
    pub max_input: usize,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning {
            max_sessions: 16,
            high_water: 1 << 20,
            stall_timeout: Duration::from_secs(5),
            drain_quiet: Duration::from_millis(250),
            fast_fail: Duration::from_secs(3),
            max_input: 1 << 20,
        }
    }
}

const FALLBACK_NOTE: &str =
    "\r\n\x1b[2m[VLINUX] The distribution's default shell could not be started; opening /bin/sh instead.\x1b[0m\r\n";

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

// ---- flow control -------------------------------------------------------------------------

struct Flow {
    pending: Mutex<usize>,
    cv: Condvar,
}

impl Flow {
    fn new() -> Self {
        Flow {
            pending: Mutex::new(0),
            cv: Condvar::new(),
        }
    }
    fn add(&self, n: usize) {
        let mut g = lock(&self.pending);
        *g = g.saturating_add(n);
    }
    fn ack(&self, n: usize) {
        let mut g = lock(&self.pending);
        *g = g.saturating_sub(n);
        self.cv.notify_all();
    }
    fn wake(&self) {
        self.cv.notify_all();
    }
    /// Blocks while at least `high` bytes are unacknowledged.
    fn wait_below(&self, high: usize, stall: Duration, closing: &AtomicBool, activity: &Activity) {
        let start = Instant::now();
        let mut g = lock(&self.pending);
        while *g >= high && !closing.load(Ordering::SeqCst) {
            if start.elapsed() >= stall {
                *g = 0;
                break;
            }
            activity.touch();
            g = self
                .cv
                .wait_timeout(g, Duration::from_millis(50))
                .map(|(g, _)| g)
                .unwrap_or_else(|p| p.into_inner().0);
        }
    }
}

struct Activity {
    base: Instant,
    last_ms: AtomicU64,
}

impl Activity {
    fn new() -> Self {
        Activity {
            base: Instant::now(),
            last_ms: AtomicU64::new(0),
        }
    }
    fn touch(&self) {
        self.last_ms
            .store(self.base.elapsed().as_millis() as u64, Ordering::SeqCst);
    }
    fn idle(&self) -> Duration {
        let now = self.base.elapsed().as_millis() as u64;
        Duration::from_millis(now.saturating_sub(self.last_ms.load(Ordering::SeqCst)))
    }
}

// ---- session ------------------------------------------------------------------------------

struct Io {
    input: Sender<Vec<u8>>,
    control: Arc<dyn PtyControl>,
}

struct Session {
    id: String,
    distro: String,
    label: String,
    started_at: u64,
    plan: LaunchPlan,
    backend: Arc<dyn PtyBackend>,
    sink: Arc<dyn TerminalSink>,
    tuning: Tuning,
    io: Mutex<Option<Io>>,
    state: Mutex<SessionState>,
    attempt: AtomicUsize,
    size: Mutex<PtySize>,
    /// Set once a line was submitted by the user (not by terminal auto-replies).
    input_submitted: AtomicBool,
    closing: AtomicBool,
    flow: Flow,
}

fn spawn_thread<F: FnOnce() + Send + 'static>(name: &str, f: F) -> Result<(), PtyError> {
    thread::Builder::new()
        .name(format!("vlinux-term-{name}"))
        .spawn(f)
        .map(|_| ())
        .map_err(|e| PtyError(format!("could not start a worker thread: {e}")))
}

impl Session {
    fn is_live(&self) -> bool {
        !self.closing.load(Ordering::SeqCst) && matches!(*lock(&self.state), SessionState::Running)
    }

    fn info(&self) -> SessionInfo {
        let idx = self
            .attempt
            .load(Ordering::SeqCst)
            .min(self.plan.attempts.len().saturating_sub(1));
        SessionInfo {
            id: self.id.clone(),
            distro: self.distro.clone(),
            label: self.label.clone(),
            state: lock(&self.state).clone(),
            started_at: self.started_at,
            shell: self
                .plan
                .attempts
                .get(idx)
                .map(|a| a.label.clone())
                .unwrap_or_default(),
        }
    }

    fn set_state(&self, state: SessionState) {
        *lock(&self.state) = state.clone();
        self.sink.state(&self.id, &state);
    }

    fn start_attempt(self: &Arc<Self>, idx: usize) -> Result<(), PtyError> {
        let mut spec = self.plan.attempts[idx].spec.clone();
        spec.size = *lock(&self.size);
        let handles = self.backend.spawn(&spec)?;

        let control = handles.control.clone();
        let (input_tx, input_rx) = mpsc::channel::<Vec<u8>>();
        let activity = Arc::new(Activity::new());
        let (done_tx, done_rx) = mpsc::channel::<()>();
        let started = Instant::now();

        // Publish the attempt *before* any thread can observe the process exiting, so a very fast
        // failure that starts the fallback cannot be overwritten by this attempt's bookkeeping.
        self.attempt.store(idx, Ordering::SeqCst);
        *lock(&self.io) = Some(Io {
            input: input_tx,
            control: control.clone(),
        });

        // From here on, any failure must not leave a running child behind.
        let abort = |e: PtyError| {
            *lock(&self.io) = None;
            let _ = control.kill();
            control.close();
            e
        };

        let mut writer = handles.writer;
        spawn_thread("in", move || {
            for chunk in input_rx {
                if writer.write_all(&chunk).is_err() || writer.flush().is_err() {
                    break;
                }
            }
        })
        .map_err(abort)?;

        let me = self.clone();
        let act = activity.clone();
        let mut reader = handles.reader;
        spawn_thread("out", move || {
            let mut buf = vec![0u8; 32 * 1024];
            loop {
                me.flow.wait_below(
                    me.tuning.high_water,
                    me.tuning.stall_timeout,
                    &me.closing,
                    &act,
                );
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        act.touch();
                        // After the user closed the session the output is only drained (so the PTY
                        // can be released) and no longer reported.
                        if !me.closing.load(Ordering::SeqCst) {
                            me.flow.add(n);
                            me.sink.output(&me.id, &buf[..n]);
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
            let _ = done_tx.send(());
        })
        .map_err(abort)?;

        let me = self.clone();
        let wait_control = control.clone();
        let mut child = handles.child;
        let act = activity;
        spawn_thread("wait", move || {
            let result = child.wait();
            let tuning = me.tuning;
            let begun = Instant::now();
            let mut released_at: Option<Instant> = None;
            loop {
                match done_rx.recv_timeout(Duration::from_millis(20)) {
                    Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
                    Err(RecvTimeoutError::Timeout) => {
                        match released_at {
                            None => {
                                if act.idle() >= tuning.drain_quiet {
                                    // Windows: the output pipe only ends once the PTY is released.
                                    wait_control.close();
                                    released_at = Some(Instant::now());
                                }
                            }
                            Some(t) if t.elapsed() >= Duration::from_secs(3) => break,
                            Some(_) => {}
                        }
                        if begun.elapsed() >= Duration::from_secs(120) {
                            break;
                        }
                    }
                }
            }
            wait_control.close();
            me.on_exit(idx, started, result);
        })
        .map_err(abort)?;

        Ok(())
    }

    fn on_exit(self: &Arc<Self>, idx: usize, started: Instant, result: io::Result<u32>) {
        if self.closing.load(Ordering::SeqCst) {
            *lock(&self.state) = SessionState::Exited { code: None };
            return;
        }
        let code = result.as_ref().ok().map(|c| *c as i32);
        let failed_fast = matches!(code, Some(c) if c != 0)
            && started.elapsed() < self.tuning.fast_fail
            && !self.input_submitted.load(Ordering::SeqCst);
        if failed_fast && idx + 1 < self.plan.attempts.len() {
            self.sink.output(&self.id, FALLBACK_NOTE.as_bytes());
            if let Err(e) = self.start_attempt(idx + 1) {
                self.set_state(SessionState::Failed {
                    message: format!("could not start a shell: {e}"),
                });
            }
            return;
        }
        match result {
            Ok(_) => self.set_state(SessionState::Exited { code }),
            Err(e) => self.set_state(SessionState::Failed {
                message: format!("lost track of the shell process: {e}"),
            }),
        }
    }

    fn shutdown(&self) {
        self.closing.store(true, Ordering::SeqCst);
        self.flow.wake();
        let io = lock(&self.io).take();
        if let Some(io) = io {
            // Only the process VLINUX started is ended. Errors (already gone) are not interesting.
            let _ = io.control.kill();
            drop(io.input);
            // Release the PTY too, off-thread: on some Windows builds closing a pseudoconsole waits
            // for its output to be drained, and the reader thread is draining it.
            let control = io.control.clone();
            let _ = thread::Builder::new()
                .name("vlinux-term-close".into())
                .spawn(move || control.close());
        }
    }
}

// ---- manager ------------------------------------------------------------------------------

pub struct TerminalManager {
    backend: Arc<dyn PtyBackend>,
    sink: Arc<dyn TerminalSink>,
    tuning: Tuning,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    next_id: AtomicU64,
}

impl TerminalManager {
    pub fn new(backend: Arc<dyn PtyBackend>, sink: Arc<dyn TerminalSink>) -> Self {
        Self::with_tuning(backend, sink, Tuning::default())
    }

    pub fn with_tuning(
        backend: Arc<dyn PtyBackend>,
        sink: Arc<dyn TerminalSink>,
        tuning: Tuning,
    ) -> Self {
        TerminalManager {
            backend,
            sink,
            tuning,
            sessions: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    /// Open a terminal in an already validated distribution (see `launch::resolve_target`).
    pub fn open_target(
        &self,
        target: &Target,
        programs: &Programs,
        size: PtySize,
    ) -> Result<SessionInfo, TerminalError> {
        let plan = build_plan(programs, target)?;
        self.open_plan(&target.name, &target.label, plan, size)
    }

    /// Open a session from an explicit plan. Used by [`open_target`](Self::open_target) and by
    /// tests that run an ordinary program under a real PTY.
    pub fn open_plan(
        &self,
        distro: &str,
        label: &str,
        plan: LaunchPlan,
        size: PtySize,
    ) -> Result<SessionInfo, TerminalError> {
        if plan.attempts.is_empty() {
            return Err(TerminalError::StartFailed {
                detail: "no way to start a shell was provided".into(),
            });
        }
        {
            let live = lock(&self.sessions)
                .values()
                .filter(|s| s.is_live())
                .count();
            if live >= self.tuning.max_sessions {
                return Err(TerminalError::TooManySessions {
                    max: self.tuning.max_sessions,
                });
            }
        }
        let id = format!("t{}", self.next_id.fetch_add(1, Ordering::SeqCst));
        let session = Arc::new(Session {
            id: id.clone(),
            distro: distro.to_string(),
            label: label.to_string(),
            started_at: unix_now(),
            plan,
            backend: self.backend.clone(),
            sink: self.sink.clone(),
            tuning: self.tuning,
            io: Mutex::new(None),
            state: Mutex::new(SessionState::Running),
            attempt: AtomicUsize::new(0),
            size: Mutex::new(size),
            input_submitted: AtomicBool::new(false),
            closing: AtomicBool::new(false),
            flow: Flow::new(),
        });
        session
            .start_attempt(0)
            .map_err(|e| TerminalError::StartFailed {
                detail: e.to_string(),
            })?;
        lock(&self.sessions).insert(id, session.clone());
        Ok(session.info())
    }

    fn get(&self, id: &str) -> Result<Arc<Session>, TerminalError> {
        lock(&self.sessions)
            .get(id)
            .cloned()
            .ok_or_else(|| TerminalError::UnknownSession { id: id.to_string() })
    }

    /// Forward keystrokes (or pasted text) to the PTY. Never blocks.
    pub fn write(&self, id: &str, data: &[u8]) -> Result<(), TerminalError> {
        if data.len() > self.tuning.max_input {
            return Err(TerminalError::InputTooLarge {
                len: data.len(),
                max: self.tuning.max_input,
            });
        }
        let s = self.get(id)?;
        if !s.is_live() {
            return Err(TerminalError::SessionEnded { id: id.to_string() });
        }
        if data.contains(&b'\r') || data.contains(&b'\n') {
            s.input_submitted.store(true, Ordering::SeqCst);
        }
        let g = lock(&s.io);
        let io = g
            .as_ref()
            .ok_or_else(|| TerminalError::SessionEnded { id: id.to_string() })?;
        io.input
            .send(data.to_vec())
            .map_err(|_| TerminalError::SessionEnded { id: id.to_string() })
    }

    pub fn resize(&self, id: &str, rows: u32, cols: u32) -> Result<(), TerminalError> {
        let size = PtySize::checked(rows, cols).ok_or(TerminalError::InvalidSize { rows, cols })?;
        let s = self.get(id)?;
        {
            let mut cur = lock(&s.size);
            if *cur == size {
                return Ok(());
            }
            *cur = size;
        }
        if !s.is_live() {
            return Ok(()); // remembered; nothing to resize
        }
        let control = lock(&s.io).as_ref().map(|io| io.control.clone());
        if let Some(c) = control {
            c.resize(size).map_err(|e| TerminalError::StartFailed {
                detail: format!("could not resize the terminal: {e}"),
            })?;
        }
        Ok(())
    }

    /// The UI reports it has processed `bytes` of output (flow control).
    pub fn ack(&self, id: &str, bytes: usize) {
        if let Ok(s) = self.get(id) {
            s.flow.ack(bytes);
        }
    }

    /// Close one session. Other sessions, WSL and the distribution are not affected.
    pub fn close(&self, id: &str) -> Result<(), TerminalError> {
        let s = lock(&self.sessions)
            .remove(id)
            .ok_or_else(|| TerminalError::UnknownSession { id: id.to_string() })?;
        s.shutdown();
        Ok(())
    }

    pub fn close_all(&self) {
        let all: Vec<Arc<Session>> = lock(&self.sessions).drain().map(|(_, s)| s).collect();
        for s in all {
            s.shutdown();
        }
    }

    pub fn list(&self) -> Vec<SessionInfo> {
        let mut v: Vec<SessionInfo> = lock(&self.sessions).values().map(|s| s.info()).collect();
        v.sort_by_key(|i| i.id.trim_start_matches('t').parse::<u64>().unwrap_or(0));
        v
    }

    pub fn live_count(&self) -> usize {
        lock(&self.sessions)
            .values()
            .filter(|s| s.is_live())
            .count()
    }
}

impl Drop for TerminalManager {
    fn drop(&mut self) {
        self.close_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DistroKind, DistroState, RuntimeStatus};
    use crate::terminal::launch::{resolve_target, LaunchAttempt};
    use crate::terminal::pty::PtySpawn;
    use crate::terminal::testing::{distro, report, Collector, FakeBackend};

    const T: Duration = Duration::from_secs(10);

    fn quick() -> Tuning {
        Tuning {
            drain_quiet: Duration::from_millis(40),
            fast_fail: Duration::from_secs(5),
            ..Tuning::default()
        }
    }

    fn fixture(
        tuning: Tuning,
        backend: FakeBackend,
    ) -> (Arc<TerminalManager>, FakeBackend, Arc<Collector>) {
        let sink = Collector::new();
        let m = Arc::new(TerminalManager::with_tuning(
            Arc::new(backend.clone()),
            sink.clone(),
            tuning,
        ));
        (m, backend, sink)
    }

    fn plan() -> LaunchPlan {
        LaunchPlan::single("test shell", "fake-shell", &["-i"])
    }

    fn two_attempt_plan() -> LaunchPlan {
        let mk = |label: &str, args: &[&str]| LaunchAttempt {
            label: label.into(),
            spec: PtySpawn {
                program: "fake".into(),
                args: args.iter().map(|s| s.to_string()).collect(),
                env: vec![],
                cwd: None,
                size: PtySize::DEFAULT,
            },
        };
        LaunchPlan {
            attempts: vec![mk("default shell", &["a"]), mk("/bin/sh", &["b"])],
        }
    }

    fn open(m: &TerminalManager) -> SessionInfo {
        m.open_plan("Debian", "Debian GNU/Linux", plan(), PtySize::DEFAULT)
            .unwrap()
    }

    // ---- creation -----------------------------------------------------------------------

    #[test]
    fn creating_a_session_starts_exactly_one_process_and_reports_it() {
        let (m, b, _s) = fixture(quick(), FakeBackend::new());
        assert_eq!(b.proc_count(), 0, "nothing runs until a session is opened");
        let info = open(&m);
        assert_eq!(info.id, "t1");
        assert_eq!(info.distro, "Debian");
        assert_eq!(info.label, "Debian GNU/Linux");
        assert_eq!(info.state, SessionState::Running);
        assert_eq!(info.shell, "test shell");
        assert_eq!(b.proc_count(), 1);
        assert_eq!(m.list(), vec![info]);
        assert_eq!(m.live_count(), 1);
    }

    #[test]
    fn session_ids_are_unique() {
        let (m, _b, _s) = fixture(quick(), FakeBackend::new());
        let a = open(&m);
        let b = open(&m);
        assert_ne!(a.id, b.id);
        assert_eq!(m.list().len(), 2);
    }

    #[test]
    fn process_startup_failure_is_reported_and_leaves_nothing_behind() {
        let (m, b, _s) = fixture(quick(), FakeBackend::new());
        b.fail_next_spawn("could not start 'wsl.exe': not found");
        let err = m
            .open_plan("Debian", "Debian", plan(), PtySize::DEFAULT)
            .unwrap_err();
        match &err {
            TerminalError::StartFailed { detail } => assert!(detail.contains("wsl.exe")),
            other => panic!("unexpected {other:?}"),
        }
        assert!(m.list().is_empty());
        assert_eq!(m.live_count(), 0);
        // The failure did not consume a slot or wedge the manager.
        b.succeed_next_spawn();
        assert!(m
            .open_plan("Debian", "Debian", plan(), PtySize::DEFAULT)
            .is_ok());
    }

    #[test]
    fn an_empty_plan_is_refused() {
        let (m, _b, _s) = fixture(quick(), FakeBackend::new());
        let e = m
            .open_plan(
                "Debian",
                "Debian",
                LaunchPlan { attempts: vec![] },
                PtySize::DEFAULT,
            )
            .unwrap_err();
        assert!(matches!(e, TerminalError::StartFailed { .. }));
    }

    #[test]
    fn the_session_limit_counts_live_sessions_only() {
        let (m, b, s) = fixture(
            Tuning {
                max_sessions: 2,
                ..quick()
            },
            FakeBackend::new(),
        );
        let first = open(&m);
        let _second = open(&m);
        assert!(matches!(
            m.open_plan("Debian", "D", plan(), PtySize::DEFAULT),
            Err(TerminalError::TooManySessions { max: 2 })
        ));
        // A finished session no longer counts.
        b.proc(0).unwrap().exit(0);
        assert!(s
            .wait_state(&first.id, T, |st| matches!(st, SessionState::Exited { .. }))
            .is_some());
        assert!(m.open_plan("Debian", "D", plan(), PtySize::DEFAULT).is_ok());
    }

    // ---- opening a distribution ---------------------------------------------------------

    #[test]
    fn opening_a_target_runs_wsl_exe_for_exactly_that_distribution() {
        let (m, b, _s) = fixture(quick(), FakeBackend::new());
        let r = report(
            RuntimeStatus::Ready,
            vec![
                distro("Debian", DistroState::Stopped, DistroKind::Regular),
                distro("archlinux", DistroState::Running, DistroKind::Regular),
            ],
        );
        let target = resolve_target(&r, None, Some("archlinux")).unwrap();
        let info = m
            .open_target(
                &target,
                &Programs::fake(),
                PtySize {
                    rows: 40,
                    cols: 120,
                },
            )
            .unwrap();
        assert_eq!(info.distro, "archlinux");
        let specs = b.specs();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].program, "wsl.exe");
        assert_eq!(specs[0].args, ["-d", "archlinux", "--cd", "~"]);
        assert_eq!(
            specs[0].size,
            PtySize {
                rows: 40,
                cols: 120
            }
        );
        assert!(specs[0]
            .env
            .iter()
            .any(|(k, v)| k == "TERM" && v == "xterm-256color"));
    }

    #[test]
    fn invalid_missing_or_unavailable_targets_never_reach_the_backend() {
        let (_m, b, _s) = fixture(quick(), FakeBackend::new());
        let ready = report(
            RuntimeStatus::Ready,
            vec![distro("Debian", DistroState::Running, DistroKind::Regular)],
        );
        assert!(resolve_target(&ready, Some("Debian; calc"), None).is_err());
        assert!(resolve_target(&ready, Some("Fedora"), None).is_err());
        assert!(resolve_target(
            &report(RuntimeStatus::NotInstalled, vec![]),
            Some("Debian"),
            None
        )
        .is_err());
        assert!(
            resolve_target(&report(RuntimeStatus::NoDistributions, vec![]), None, None).is_err()
        );
        assert_eq!(
            b.proc_count(),
            0,
            "validation failures must not start any process"
        );
    }

    // ---- input ---------------------------------------------------------------------------

    #[test]
    fn keystrokes_are_forwarded_to_the_pty_in_order_and_verbatim() {
        let (m, b, _s) = fixture(quick(), FakeBackend::new());
        let info = open(&m);
        let p = b.proc(0).unwrap();
        m.write(&info.id, b"echo h").unwrap();
        m.write(&info.id, "é✓日本".as_bytes()).unwrap();
        m.write(&info.id, b"\x1b[A").unwrap(); // arrow up is just bytes: history is the shell's job
        m.write(&info.id, b"\x03").unwrap(); // Ctrl+C
        m.write(&info.id, b"\r").unwrap();
        let expected: Vec<u8> = [
            b"echo h".as_slice(),
            "é✓日本".as_bytes(),
            b"\x1b[A",
            b"\x03",
            b"\r",
        ]
        .concat();
        assert!(p.wait_until(T, |p| p.written().len() >= expected.len()));
        assert_eq!(p.written(), expected);
    }

    #[test]
    fn input_errors_are_reported_not_swallowed() {
        let (m, b, _s) = fixture(
            Tuning {
                max_input: 16,
                ..quick()
            },
            FakeBackend::new(),
        );
        assert!(matches!(
            m.write("nope", b"x"),
            Err(TerminalError::UnknownSession { .. })
        ));
        let info = open(&m);
        assert!(matches!(
            m.write(&info.id, &[b'x'; 17]),
            Err(TerminalError::InputTooLarge { len: 17, max: 16 })
        ));
        assert!(m.write(&info.id, &[]).is_ok());
        // After the process ended, input is refused.
        b.proc(0).unwrap().exit(0);
        let end = Instant::now() + T;
        while m.live_count() > 0 && Instant::now() < end {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(matches!(
            m.write(&info.id, b"x"),
            Err(TerminalError::SessionEnded { .. })
        ));
    }

    #[test]
    fn a_line_discipline_round_trip_works_end_to_end() {
        let (m, _b, s) = fixture(quick(), FakeBackend::new().with_echo());
        let info = open(&m);
        m.write(&info.id, b"echo hi\r").unwrap();
        assert!(s.wait_text(&info.id, "echo hi\r", T));
    }

    // ---- output --------------------------------------------------------------------------

    #[test]
    fn output_is_streamed_in_order() {
        let (m, b, s) = fixture(quick(), FakeBackend::new());
        let info = open(&m);
        let p = b.proc(0).unwrap();
        for i in 0..200 {
            p.output(format!("line {i}\r\n").as_bytes());
        }
        assert!(s.wait_text(&info.id, "line 199\r\n", T));
        let expected: String = (0..200).map(|i| format!("line {i}\r\n")).collect();
        assert_eq!(s.text(&info.id), expected);
    }

    #[test]
    fn unicode_and_ansi_pass_through_unchanged_even_when_split_mid_character() {
        let (m, b, s) = fixture(quick(), FakeBackend::new());
        let info = open(&m);
        let p = b.proc(0).unwrap();
        let text = "\x1b[1;32mhéllo ✓ 日本語 🦀\x1b[0m\r\n";
        let bytes = text.as_bytes();
        // Cut after every single byte: the worst case for multi-byte characters.
        for b in bytes.chunks(1) {
            p.output(b);
        }
        assert!(s.wait_output(&info.id, T, |o| o.len() >= bytes.len()));
        assert_eq!(s.bytes(&info.id), bytes);
        assert_eq!(String::from_utf8(s.bytes(&info.id)).unwrap(), text);
    }

    #[test]
    fn two_sessions_never_mix_their_output_or_input() {
        let (m, b, s) = fixture(quick(), FakeBackend::new());
        let a = open(&m);
        let c = open(&m);
        b.proc(0).unwrap().output(b"from-a");
        b.proc(1).unwrap().output(b"from-b");
        assert!(s.wait_text(&a.id, "from-a", T));
        assert!(s.wait_text(&c.id, "from-b", T));
        assert_eq!(s.text(&a.id), "from-a");
        assert_eq!(s.text(&c.id), "from-b");
        m.write(&a.id, b"A").unwrap();
        m.write(&c.id, b"B").unwrap();
        assert!(b.proc(0).unwrap().wait_until(T, |p| p.written() == b"A"));
        assert!(b.proc(1).unwrap().wait_until(T, |p| p.written() == b"B"));
    }

    // ---- large output & flow control -------------------------------------------------------

    fn pattern(total: usize) -> Vec<u8> {
        (0..total).map(|i| (i % 251) as u8).collect()
    }

    #[test]
    fn large_output_is_paused_when_the_ui_falls_behind_and_resumes_on_acknowledgement() {
        let high = 64 * 1024;
        let (m, b, s) = fixture(
            Tuning {
                high_water: high,
                stall_timeout: Duration::from_secs(60),
                ..quick()
            },
            FakeBackend::new(),
        );
        let info = open(&m);
        let p = b.proc(0).unwrap();
        let total = 4 * 1024 * 1024;
        let data = pattern(total);
        for c in data.chunks(16 * 1024) {
            p.output(c);
        }
        // Nobody acknowledges: the reader must stop shortly after the high-water mark.
        thread::sleep(Duration::from_millis(300));
        let held = s.len(&info.id);
        assert!(
            held >= high,
            "should have delivered up to the mark ({held})"
        );
        assert!(
            held < high + 64 * 1024,
            "reader must pause near the high-water mark, got {held} of {total}"
        );
        thread::sleep(Duration::from_millis(200));
        assert_eq!(s.len(&info.id), held, "no more output while unacknowledged");

        // The UI catches up: everything arrives, intact.
        let end = Instant::now() + Duration::from_secs(30);
        let mut acked = 0usize;
        while s.len(&info.id) < total && Instant::now() < end {
            let now = s.len(&info.id);
            if now > acked {
                m.ack(&info.id, now - acked);
                acked = now;
            }
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(s.len(&info.id), total);
        assert_eq!(s.bytes(&info.id), data);
        assert!(
            s.max_chunk() <= 32 * 1024,
            "chunks stay small so the UI can stay responsive"
        );
    }

    #[test]
    fn a_ui_that_stops_acknowledging_cannot_hang_a_session_forever() {
        let (m, b, s) = fixture(
            Tuning {
                high_water: 16 * 1024,
                stall_timeout: Duration::from_millis(150),
                ..quick()
            },
            FakeBackend::new(),
        );
        let info = open(&m);
        let p = b.proc(0).unwrap();
        let data = pattern(256 * 1024);
        for c in data.chunks(8 * 1024) {
            p.output(c);
        }
        assert!(s.wait_output(&info.id, Duration::from_secs(30), |o| o.len() == data.len()));
        assert_eq!(s.bytes(&info.id), data);
    }

    // ---- resize ---------------------------------------------------------------------------

    #[test]
    fn resize_reaches_the_pty_and_bad_sizes_are_rejected() {
        let (m, b, _s) = fixture(quick(), FakeBackend::new());
        let info = open(&m);
        let p = b.proc(0).unwrap();
        assert_eq!(p.sizes(), vec![PtySize::DEFAULT]);
        m.resize(&info.id, 40, 120).unwrap();
        m.resize(&info.id, 40, 120).unwrap(); // unchanged: not sent again
        m.resize(&info.id, 24, 80).unwrap();
        assert_eq!(
            p.sizes(),
            vec![
                PtySize::DEFAULT,
                PtySize {
                    rows: 40,
                    cols: 120
                },
                PtySize { rows: 24, cols: 80 }
            ]
        );
        assert!(matches!(
            m.resize(&info.id, 0, 80),
            Err(TerminalError::InvalidSize { .. })
        ));
        assert!(matches!(
            m.resize(&info.id, 24, 0),
            Err(TerminalError::InvalidSize { .. })
        ));
        assert!(matches!(
            m.resize(&info.id, 5000, 80),
            Err(TerminalError::InvalidSize { .. })
        ));
        assert!(matches!(
            m.resize("nope", 24, 80),
            Err(TerminalError::UnknownSession { .. })
        ));
        assert_eq!(p.sizes().len(), 3);
    }

    #[test]
    fn the_latest_size_is_used_if_a_fallback_shell_has_to_be_started() {
        let (m, b, _s) = fixture(quick(), FakeBackend::new());
        let info = m
            .open_plan("Debian", "D", two_attempt_plan(), PtySize::DEFAULT)
            .unwrap();
        m.resize(&info.id, 50, 150).unwrap();
        b.proc(0).unwrap().exit(1);
        let second = b.wait_proc(1, T).expect("fallback started");
        assert_eq!(
            second.spec.size,
            PtySize {
                rows: 50,
                cols: 150
            }
        );
    }

    // ---- termination ------------------------------------------------------------------------

    #[test]
    fn shell_exit_is_reported_with_its_code_after_all_output_was_delivered() {
        let (m, b, s) = fixture(quick(), FakeBackend::new());
        let info = open(&m);
        let p = b.proc(0).unwrap();
        p.output(b"logout\r\n");
        p.exit(3);
        let st = s
            .wait_state(&info.id, T, |st| matches!(st, SessionState::Exited { .. }))
            .expect("exit reported");
        assert_eq!(st, SessionState::Exited { code: Some(3) });
        assert_eq!(
            s.text(&info.id),
            "logout\r\n",
            "output must not be lost to the exit"
        );
        assert_eq!(m.list()[0].state, SessionState::Exited { code: Some(3) });
        assert_eq!(s.states(&info.id).len(), 1, "exactly one state report");
    }

    #[test]
    fn exit_is_detected_even_when_the_output_pipe_stays_open_like_conpty() {
        let (m, b, s) = fixture(quick(), FakeBackend::new().windows_style());
        let info = open(&m);
        let p = b.proc(0).unwrap();
        p.output(b"bye\r\n");
        p.exit(0);
        let st = s
            .wait_state(&info.id, T, |st| matches!(st, SessionState::Exited { .. }))
            .expect("exit reported");
        assert_eq!(st, SessionState::Exited { code: Some(0) });
        assert_eq!(s.text(&info.id), "bye\r\n");
        assert!(
            p.wait_until(T, |p| p.was_closed()),
            "the PTY must be released after the process ends"
        );
    }

    #[test]
    fn the_pty_is_released_after_the_process_dies_on_its_own() {
        for backend in [FakeBackend::new(), FakeBackend::new().windows_style()] {
            let (m, b, s) = fixture(quick(), backend);
            let info = open(&m);
            let p = b.proc(0).unwrap();
            p.exit(137); // killed from outside, e.g. `wsl --shutdown` by the user
            assert!(s
                .wait_state(&info.id, T, |st| matches!(
                    st,
                    SessionState::Exited { code: Some(137) }
                ))
                .is_some());
            assert!(p.wait_until(T, |p| p.was_closed()));
        }
    }

    #[test]
    fn closing_a_session_ends_only_that_session() {
        let (m, b, s) = fixture(quick(), FakeBackend::new());
        let one = open(&m);
        let two = open(&m);
        m.close(&one.id).unwrap();
        let p0 = b.proc(0).unwrap();
        let p1 = b.proc(1).unwrap();
        assert!(p0.was_killed());
        assert!(
            p0.wait_until(T, |p| p.was_closed()),
            "closed session's PTY is released"
        );
        assert!(!p1.was_killed(), "the other session must be untouched");
        assert!(!p1.was_closed());
        assert_eq!(m.list().len(), 1);
        assert_eq!(m.list()[0].id, two.id);
        assert!(matches!(
            m.write(&one.id, b"x"),
            Err(TerminalError::UnknownSession { .. })
        ));
        assert!(matches!(
            m.close(&one.id),
            Err(TerminalError::UnknownSession { .. })
        ));
        // ...and it keeps working.
        p1.output(b"still here");
        assert!(s.wait_text(&two.id, "still here", T));
        m.write(&two.id, b"ok").unwrap();
        assert!(p1.wait_until(T, |p| p.written() == b"ok"));
        // A user-closed session does not report an "exited" state to a UI that already forgot it.
        assert!(s.states(&one.id).is_empty());
    }

    #[test]
    fn closing_works_while_output_is_flooding_and_nobody_acknowledges() {
        let (m, b, _s) = fixture(
            Tuning {
                high_water: 8 * 1024,
                stall_timeout: Duration::from_secs(60),
                ..quick()
            },
            FakeBackend::new(),
        );
        let info = open(&m);
        let p = b.proc(0).unwrap();
        for _ in 0..64 {
            p.output(&[b'x'; 16 * 1024]);
        }
        thread::sleep(Duration::from_millis(100));
        m.close(&info.id).unwrap();
        assert!(
            p.wait_until(T, |p| p.was_killed() && p.was_closed()),
            "close must not wait for the UI"
        );
    }

    #[test]
    fn close_all_and_drop_end_every_process_and_nothing_else() {
        let (m, b, _s) = fixture(quick(), FakeBackend::new());
        open(&m);
        open(&m);
        m.close_all();
        assert!(m.list().is_empty());
        assert!(b.proc(0).unwrap().was_killed() && b.proc(1).unwrap().was_killed());

        let (m2, b2, _s2) = fixture(quick(), FakeBackend::new());
        open(&m2);
        drop(m2);
        assert!(
            b2.proc(0).unwrap().was_killed(),
            "dropping the manager must not leak a shell"
        );
    }

    #[test]
    fn closing_never_issues_any_wsl_management_command() {
        // Session cleanup only kills the process VLINUX started. The backend is never asked to run
        // anything else, so WSL (`--shutdown`, `--terminate`, `--unregister`) cannot be touched.
        let (m, b, _s) = fixture(quick(), FakeBackend::new());
        let r = report(
            RuntimeStatus::Ready,
            vec![distro("Debian", DistroState::Running, DistroKind::Regular)],
        );
        let t = resolve_target(&r, Some("Debian"), None).unwrap();
        let info = m
            .open_target(&t, &Programs::fake(), PtySize::DEFAULT)
            .unwrap();
        m.close(&info.id).unwrap();
        m.close_all();
        assert_eq!(
            b.specs().len(),
            1,
            "only the one terminal process was ever started"
        );
        assert_eq!(b.specs()[0].args, ["-d", "Debian", "--cd", "~"]);
    }

    // ---- shell fallback ---------------------------------------------------------------------

    #[test]
    fn an_immediate_startup_failure_falls_back_to_the_next_shell_and_says_so() {
        let (m, b, s) = fixture(quick(), FakeBackend::new());
        let info = m
            .open_plan("Debian", "D", two_attempt_plan(), PtySize::DEFAULT)
            .unwrap();
        b.proc(0).unwrap().exit(1);
        let second = b.wait_proc(1, T).expect("fallback shell started");
        assert_eq!(second.spec.args, ["b"]);
        assert!(s.wait_text(&info.id, "opening /bin/sh instead", T));
        assert_eq!(m.list()[0].state, SessionState::Running);
        assert_eq!(m.list()[0].shell, "/bin/sh");
        assert!(
            s.states(&info.id).is_empty(),
            "the session never ended, so no exit is reported"
        );
        // The fallback shell is fully usable.
        m.write(&info.id, b"ls\r").unwrap();
        assert!(second.wait_until(T, |p| p.written() == b"ls\r"));
        second.exit(0);
        assert!(s
            .wait_state(&info.id, T, |st| matches!(
                st,
                SessionState::Exited { code: Some(0) }
            ))
            .is_some());
    }

    #[test]
    fn a_successful_or_late_exit_never_triggers_the_fallback() {
        // exit code 0
        let (m, b, s) = fixture(quick(), FakeBackend::new());
        let info = m
            .open_plan("Debian", "D", two_attempt_plan(), PtySize::DEFAULT)
            .unwrap();
        b.proc(0).unwrap().exit(0);
        assert!(s
            .wait_state(&info.id, T, |st| matches!(
                st,
                SessionState::Exited { code: Some(0) }
            ))
            .is_some());
        assert_eq!(b.proc_count(), 1);

        // non-zero, but after the fast-fail window
        let (m, b, s) = fixture(
            Tuning {
                fast_fail: Duration::from_millis(1),
                ..quick()
            },
            FakeBackend::new(),
        );
        let info = m
            .open_plan("Debian", "D", two_attempt_plan(), PtySize::DEFAULT)
            .unwrap();
        thread::sleep(Duration::from_millis(30));
        b.proc(0).unwrap().exit(2);
        assert!(s
            .wait_state(&info.id, T, |st| matches!(
                st,
                SessionState::Exited { code: Some(2) }
            ))
            .is_some());
        assert_eq!(b.proc_count(), 1);
    }

    #[test]
    fn a_shell_the_user_already_typed_into_is_not_replaced() {
        let (m, b, s) = fixture(quick(), FakeBackend::new());
        let info = m
            .open_plan("Debian", "D", two_attempt_plan(), PtySize::DEFAULT)
            .unwrap();
        m.write(&info.id, b"exit 1\r").unwrap();
        b.proc(0).unwrap().exit(1);
        assert!(s
            .wait_state(&info.id, T, |st| matches!(
                st,
                SessionState::Exited { code: Some(1) }
            ))
            .is_some());
        assert_eq!(
            b.proc_count(),
            1,
            "the user's own `exit 1` must not start another shell"
        );
    }

    #[test]
    fn terminal_auto_replies_do_not_count_as_user_input() {
        let (m, b, s) = fixture(quick(), FakeBackend::new());
        let info = m
            .open_plan("Debian", "D", two_attempt_plan(), PtySize::DEFAULT)
            .unwrap();
        m.write(&info.id, b"\x1b[1;1R").unwrap(); // cursor-position reply from the emulator
        b.proc(0).unwrap().exit(1);
        assert!(b.wait_proc(1, T).is_some(), "fallback should still start");
        assert!(s.wait_text(&info.id, "opening /bin/sh instead", T));
    }

    #[test]
    fn if_the_fallback_cannot_start_either_the_session_fails_cleanly() {
        let (m, b, s) = fixture(quick(), FakeBackend::new());
        let info = m
            .open_plan("Debian", "D", two_attempt_plan(), PtySize::DEFAULT)
            .unwrap();
        b.fail_next_spawn("no such program");
        b.proc(0).unwrap().exit(1);
        let st = s
            .wait_state(&info.id, T, |st| matches!(st, SessionState::Failed { .. }))
            .expect("failure reported");
        match st {
            SessionState::Failed { message } => {
                assert!(message.contains("no such program"), "{message}")
            }
            _ => unreachable!(),
        }
        assert!(matches!(
            m.write(&info.id, b"x"),
            Err(TerminalError::SessionEnded { .. })
        ));
        m.close(&info.id).unwrap();
    }

    #[test]
    fn acknowledging_an_unknown_session_is_harmless() {
        let (m, _b, _s) = fixture(quick(), FakeBackend::new());
        m.ack("nope", 100);
    }
}
