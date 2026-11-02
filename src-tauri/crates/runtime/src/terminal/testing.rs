//! Test doubles for terminal sessions. Public (not `cfg(test)`) so the integration tests in
//! `tests/` can use them; nothing here is used at runtime.
//!
//! * [`FakeBackend`] — a scriptable PTY: tests feed "process output", observe forwarded input,
//!   resizes and kills, and decide when the process exits. No process is started.
//! * [`Collector`] — a [`TerminalSink`] that records output and state changes, can answer the
//!   terminal's cursor-position request like a real emulator, and can acknowledge output.
//! * [`distro`] / [`report`] — build `WslReport`s without running detection.

use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Write};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use super::pty::{PtyBackend, PtyChild, PtyControl, PtyError, PtyHandles, PtySize, PtySpawn};
use super::session::{SessionState, TerminalManager, TerminalSink};
use crate::adapters::{AdapterInfo, PackageManager};
use crate::model::{
    Distro, DistroKind, DistroState, HostEnv, IdentitySource, Origin, RuntimeStatus, WslReport,
};

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

// ---- reports ------------------------------------------------------------------------------

pub fn distro(name: &str, state: DistroState, kind: DistroKind) -> Distro {
    Distro {
        name: name.to_string(),
        state,
        wsl_version: 2,
        is_default: false,
        kind,
        origin: Origin::Existing,
        adapter: AdapterInfo {
            id: "generic".into(),
            display_name: "Linux".into(),
            package_manager: PackageManager::Unknown,
            package_manager_name: "unknown".into(),
            rolling_release: false,
            notes: String::new(),
        },
        identity_source: IdentitySource::RegisteredName,
        os_release: None,
        display_label: format!("{name} label"),
    }
}

pub fn report(status: RuntimeStatus, distros: Vec<Distro>) -> WslReport {
    WslReport {
        status,
        host: HostEnv {
            is_windows: true,
            windows_build: Some(26100),
        },
        wsl_version: Some("2.4.13.0".into()),
        kernel_version: None,
        wslg_version: None,
        default_wsl_version: Some(2),
        wsl2_available: Some(true),
        distros,
        default_distro: None,
        suggested_distro: None,
        pending_reboot: None,
        warnings: Vec::new(),
        detected_at: 0,
    }
}

// ---- fake PTY -----------------------------------------------------------------------------

/// One fake process. Tests keep these to drive and inspect it.
pub struct FakeProc {
    pub spec: PtySpawn,
    out_tx: Mutex<Option<Sender<Vec<u8>>>>,
    written: Mutex<Vec<u8>>,
    sizes: Mutex<Vec<PtySize>>,
    exit: Mutex<Option<u32>>,
    exit_cv: Condvar,
    killed: Mutex<bool>,
    closed: Mutex<bool>,
    eof_on_exit: bool,
    echo: bool,
}

impl FakeProc {
    /// Make the process "print" `bytes`.
    pub fn output(&self, bytes: &[u8]) {
        if let Some(tx) = lock(&self.out_tx).as_ref() {
            let _ = tx.send(bytes.to_vec());
        }
    }
    /// The process ends with `code`. With `eof_on_exit` the output ends too (Unix behaviour);
    /// otherwise it ends only when the PTY is released (Windows ConPTY behaviour).
    pub fn exit(&self, code: u32) {
        *lock(&self.exit) = Some(code);
        self.exit_cv.notify_all();
        if self.eof_on_exit {
            lock(&self.out_tx).take();
        }
    }
    pub fn written(&self) -> Vec<u8> {
        lock(&self.written).clone()
    }
    pub fn written_text(&self) -> String {
        String::from_utf8_lossy(&self.written()).into_owned()
    }
    pub fn sizes(&self) -> Vec<PtySize> {
        lock(&self.sizes).clone()
    }
    pub fn was_killed(&self) -> bool {
        *lock(&self.killed)
    }
    pub fn was_closed(&self) -> bool {
        *lock(&self.closed)
    }
    pub fn wait_until<F: Fn(&FakeProc) -> bool>(&self, timeout: Duration, f: F) -> bool {
        let end = Instant::now() + timeout;
        while Instant::now() < end {
            if f(self) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        f(self)
    }
}

struct ChanReader {
    rx: Receiver<Vec<u8>>,
    left: Vec<u8>,
}

impl Read for ChanReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.left.is_empty() {
            match self.rx.recv() {
                Ok(v) => self.left = v,
                Err(_) => return Ok(0),
            }
        }
        let n = buf.len().min(self.left.len());
        buf[..n].copy_from_slice(&self.left[..n]);
        self.left.drain(..n);
        Ok(n)
    }
}

struct RecWriter(Arc<FakeProc>);

impl Write for RecWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        lock(&self.0.written).extend_from_slice(buf);
        if self.0.echo {
            self.0.output(buf);
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct FakeControl(Arc<FakeProc>);

impl PtyControl for FakeControl {
    fn resize(&self, size: PtySize) -> io::Result<()> {
        if *lock(&self.0.closed) {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"));
        }
        lock(&self.0.sizes).push(size);
        Ok(())
    }
    fn kill(&self) -> io::Result<()> {
        *lock(&self.0.killed) = true;
        self.0.exit(137);
        Ok(())
    }
    fn close(&self) {
        *lock(&self.0.closed) = true;
        lock(&self.0.out_tx).take();
    }
}

struct FakeChild(Arc<FakeProc>);

impl PtyChild for FakeChild {
    fn wait(&mut self) -> io::Result<u32> {
        let mut g = lock(&self.0.exit);
        loop {
            if let Some(c) = *g {
                return Ok(c);
            }
            g = self.0.exit_cv.wait(g).unwrap_or_else(|p| p.into_inner());
        }
    }
}

#[derive(Default)]
struct FakeInner {
    /// Per-spawn scripted failures, consumed in order (`Some(msg)` = fail this spawn).
    failures: Mutex<VecDeque<Option<String>>>,
    procs: Mutex<Vec<Arc<FakeProc>>>,
    specs: Mutex<Vec<PtySpawn>>,
}

/// A PTY backend that starts no processes.
#[derive(Clone)]
pub struct FakeBackend {
    inner: Arc<FakeInner>,
    eof_on_exit: bool,
    echo: bool,
}

impl FakeBackend {
    pub fn new() -> Self {
        FakeBackend {
            inner: Arc::new(FakeInner::default()),
            eof_on_exit: true,
            echo: false,
        }
    }
    /// Output ends only when the PTY is released (like ConPTY).
    pub fn windows_style(mut self) -> Self {
        self.eof_on_exit = false;
        self
    }
    /// Input is echoed back as output (a line discipline with ECHO).
    pub fn with_echo(mut self) -> Self {
        self.echo = true;
        self
    }
    /// The next spawn fails with `message` (queue several for several spawns; `None` = succeed).
    pub fn fail_next_spawn(&self, message: &str) {
        lock(&self.inner.failures).push_back(Some(message.to_string()));
    }
    pub fn succeed_next_spawn(&self) {
        lock(&self.inner.failures).push_back(None);
    }
    pub fn proc(&self, index: usize) -> Option<Arc<FakeProc>> {
        lock(&self.inner.procs).get(index).cloned()
    }
    pub fn proc_count(&self) -> usize {
        lock(&self.inner.procs).len()
    }
    pub fn specs(&self) -> Vec<PtySpawn> {
        lock(&self.inner.specs).clone()
    }
    /// Wait for the `index`-th process to have been started.
    pub fn wait_proc(&self, index: usize, timeout: Duration) -> Option<Arc<FakeProc>> {
        let end = Instant::now() + timeout;
        while Instant::now() < end {
            if let Some(p) = self.proc(index) {
                return Some(p);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        self.proc(index)
    }
}

impl Default for FakeBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl PtyBackend for FakeBackend {
    fn spawn(&self, spec: &PtySpawn) -> Result<PtyHandles, PtyError> {
        lock(&self.inner.specs).push(spec.clone());
        if let Some(Some(msg)) = lock(&self.inner.failures).pop_front() {
            return Err(PtyError(msg));
        }
        let (tx, rx) = mpsc::channel();
        let proc = Arc::new(FakeProc {
            spec: spec.clone(),
            out_tx: Mutex::new(Some(tx)),
            written: Mutex::new(Vec::new()),
            sizes: Mutex::new(vec![spec.size]),
            exit: Mutex::new(None),
            exit_cv: Condvar::new(),
            killed: Mutex::new(false),
            closed: Mutex::new(false),
            eof_on_exit: self.eof_on_exit,
            echo: self.echo,
        });
        lock(&self.inner.procs).push(proc.clone());
        Ok(PtyHandles {
            reader: Box::new(ChanReader {
                rx,
                left: Vec::new(),
            }),
            writer: Box::new(RecWriter(proc.clone())),
            control: Arc::new(FakeControl(proc.clone())),
            child: Box::new(FakeChild(proc)),
        })
    }
}

// ---- recording sink -----------------------------------------------------------------------

#[derive(Default)]
struct Recorded {
    output: HashMap<String, Vec<u8>>,
    states: HashMap<String, Vec<SessionState>>,
    chunks: usize,
    max_chunk: usize,
}

/// Records everything a session reports. Optionally behaves like a terminal emulator: it answers
/// the "where is the cursor?" query (`ESC [ 6 n`) that Windows' ConPTY sends at start-up, and
/// acknowledges output (flow control) when asked to.
pub struct Collector {
    rec: Mutex<Recorded>,
    cv: Condvar,
    manager: OnceLock<Weak<TerminalManager>>,
    auto_ack: bool,
}

impl Collector {
    pub fn new() -> Arc<Self> {
        Arc::new(Collector {
            rec: Mutex::new(Recorded::default()),
            cv: Condvar::new(),
            manager: OnceLock::new(),
            auto_ack: false,
        })
    }
    /// Acknowledge every chunk immediately and answer cursor-position requests.
    pub fn emulating() -> Arc<Self> {
        Arc::new(Collector {
            rec: Mutex::new(Recorded::default()),
            cv: Condvar::new(),
            manager: OnceLock::new(),
            auto_ack: true,
        })
    }
    /// Needed only for the emulating behaviours.
    pub fn attach(&self, manager: &Arc<TerminalManager>) {
        let _ = self.manager.set(Arc::downgrade(manager));
    }

    pub fn bytes(&self, id: &str) -> Vec<u8> {
        lock(&self.rec).output.get(id).cloned().unwrap_or_default()
    }
    pub fn text(&self, id: &str) -> String {
        String::from_utf8_lossy(&self.bytes(id)).into_owned()
    }
    pub fn len(&self, id: &str) -> usize {
        lock(&self.rec).output.get(id).map(|v| v.len()).unwrap_or(0)
    }
    pub fn states(&self, id: &str) -> Vec<SessionState> {
        lock(&self.rec).states.get(id).cloned().unwrap_or_default()
    }
    pub fn chunk_count(&self) -> usize {
        lock(&self.rec).chunks
    }
    pub fn max_chunk(&self) -> usize {
        lock(&self.rec).max_chunk
    }

    /// Wait until `pred(output_so_far)` holds.
    pub fn wait_output<F: Fn(&[u8]) -> bool>(&self, id: &str, timeout: Duration, pred: F) -> bool {
        let end = Instant::now() + timeout;
        let mut g = lock(&self.rec);
        loop {
            if pred(g.output.get(id).map(|v| v.as_slice()).unwrap_or(&[])) {
                return true;
            }
            let now = Instant::now();
            if now >= end {
                return false;
            }
            g = self
                .cv
                .wait_timeout(g, end - now)
                .map(|(g, _)| g)
                .unwrap_or_else(|p| p.into_inner().0);
        }
    }
    pub fn wait_text(&self, id: &str, needle: &str, timeout: Duration) -> bool {
        self.wait_output(id, timeout, |b| String::from_utf8_lossy(b).contains(needle))
    }
    /// Wait for a state change matching `pred`.
    pub fn wait_state<F: Fn(&SessionState) -> bool>(
        &self,
        id: &str,
        timeout: Duration,
        pred: F,
    ) -> Option<SessionState> {
        let end = Instant::now() + timeout;
        let mut g = lock(&self.rec);
        loop {
            if let Some(s) = g.states.get(id).and_then(|v| v.iter().find(|s| pred(s))) {
                return Some(s.clone());
            }
            let now = Instant::now();
            if now >= end {
                return None;
            }
            g = self
                .cv
                .wait_timeout(g, end - now)
                .map(|(g, _)| g)
                .unwrap_or_else(|p| p.into_inner().0);
        }
    }
}

impl TerminalSink for Collector {
    fn output(&self, id: &str, data: &[u8]) {
        {
            let mut g = lock(&self.rec);
            g.output
                .entry(id.to_string())
                .or_default()
                .extend_from_slice(data);
            g.chunks += 1;
            g.max_chunk = g.max_chunk.max(data.len());
        }
        self.cv.notify_all();
        if self.auto_ack {
            if let Some(m) = self.manager.get().and_then(|w| w.upgrade()) {
                if data.windows(4).any(|w| w == b"\x1b[6n") {
                    let _ = m.write(id, b"\x1b[1;1R");
                }
                m.ack(id, data.len());
            }
        }
    }

    fn state(&self, id: &str, state: &SessionState) {
        lock(&self.rec)
            .states
            .entry(id.to_string())
            .or_default()
            .push(state.clone());
        self.cv.notify_all();
    }
}
