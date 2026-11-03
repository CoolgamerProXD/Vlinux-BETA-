//! Compatibility diagnostics (M5).
//!
//! A diagnostics run exercises the same code paths the apps use (WSL detection, the terminal
//! session layer, the file operations, the shared-folder engine) against one distribution and
//! reports, per check: what was checked, the distribution, whether it modified anything, the
//! result, and what to do about a failure.
//!
//! Rules:
//! * **Honest evidence.** Every result carries an [`Evidence`]. In [`Mode::Simulated`] *every*
//!   executed check is stamped `simulated`: a mock can never be reported as a real WSL test. A real
//!   run stamps `real_wsl` for checks that talked to WSL and `host` for Windows-only checks. A
//!   compile or CI run is not a real-WSL test; the report says which kind of run it was.
//! * **Read-only unless permitted.** Checks that create, rename, copy, move, delete or type into a
//!   terminal need `allow_write`; without it they are `not_tested`. A stopped distribution is started
//!   only with `allow_start`.
//! * **Self-created files only.** Writes happen in `/tmp/vlinux-diag-<token>` inside the
//!   distribution. Clean-up removes only that directory, after checking the path, and always runs.
//! * **Bounded.** A total time budget and cancellation; whatever did not run is `not_tested`.
//! * **No secrets.** The copyable report is redacted (user names in Windows and Linux paths).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::exec::{CancelToken, CommandRunner};
use crate::fs::path::linux_components;
use crate::fs::{
    ops, Conflict, EntryKind, FsErrorKind, ListOptions, ListStore, Location, Progress, Runner,
    Spaces, TransferMode,
};
use crate::lifecycle::check_target;
use crate::model::{DistroKind, DistroState, RuntimeStatus, WslReport};
use crate::probe::{valid_token, Probed, Prober};
use crate::shared::engine::{self, Ctx};
use crate::shared::{parse_wsl_conf, DistroOps, MappingState, MappingStore};
use crate::storage::{
    windows_to_wsl, DriveInspector, KnownFolder, KnownFolderResolver, StorageConfig, Verdict,
};
use crate::terminal::{PtySize, SessionState, Target, TerminalManager, TerminalSink};
use crate::wsl::{Programs, WslClient};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Talking to the real WSL on this PC.
    Real,
    /// Fakes and mocks. Results say nothing about this PC.
    Simulated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Runtime,
    Terminal,
    Filesystem,
    SharedFolders,
    Desktop,
}

impl Category {
    pub const ALL: [Category; 5] = [
        Category::Runtime,
        Category::Terminal,
        Category::Filesystem,
        Category::SharedFolders,
        Category::Desktop,
    ];
    pub fn title(&self) -> &'static str {
        match self {
            Category::Runtime => "Runtime",
            Category::Terminal => "Terminal",
            Category::Filesystem => "Filesystem",
            Category::SharedFolders => "Shared folders",
            Category::Desktop => "Desktop",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Passed,
    Failed,
    /// A precondition failed or does not apply, so the check was not attempted.
    Skipped,
    /// This PC / distribution cannot do it (for example a non-Windows host).
    Unsupported,
    /// Not attempted: no permission given, cancelled, out of time, or VLINUX has no safe test.
    NotTested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Evidence {
    /// The check ran against the real WSL on this PC.
    RealWsl,
    /// The check ran on the Windows host only (no WSL involved).
    Host,
    /// The check ran against fakes. It proves nothing about this PC.
    Simulated,
    /// The check did not run.
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CheckResult {
    pub id: String,
    pub category: Category,
    pub title: String,
    /// What was checked, in one sentence.
    pub checked: String,
    pub distro: Option<String>,
    pub outcome: Outcome,
    pub evidence: Evidence,
    /// This check changed something (always inside the diagnostic directory or a throw-away
    /// terminal session). `false` for checks that did not run.
    pub modified: bool,
    pub message: String,
    pub guidance: Option<String>,
    pub ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Options {
    pub distro: Option<String>,
    /// Start the distribution if it is stopped.
    pub allow_start: bool,
    /// Allow checks that create/rename/copy/move/delete files inside the diagnostic directory and
    /// type into a throw-away terminal session.
    pub allow_write: bool,
    /// Limit to these categories (`None` = all).
    pub categories: Option<Vec<Category>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub unsupported: usize,
    pub not_tested: usize,
    /// Checks that passed *and* ran against the real WSL.
    pub real_wsl_passed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    pub generated_at: u64,
    pub mode: Mode,
    pub distro: Option<String>,
    pub allow_write: bool,
    pub allow_start: bool,
    pub results: Vec<CheckResult>,
    pub summary: Summary,
    /// Copyable plain text, redacted.
    pub text: String,
    pub cancelled: bool,
    /// Set only if clean-up could not remove the temporary folder.
    pub diag_dir: Option<String>,
}

/// Terminal output recorder for diagnostics sessions (its own sink: diagnostics sessions never
/// appear in the user's terminal tabs).
#[derive(Default)]
pub struct DiagSink {
    inner: Mutex<SinkInner>,
    cv: Condvar,
}

#[derive(Default)]
struct SinkInner {
    out: HashMap<String, Vec<u8>>,
    ended: HashMap<String, SessionState>,
}

const SINK_CAP: usize = 256 * 1024;

impl TerminalSink for DiagSink {
    fn output(&self, id: &str, data: &[u8]) {
        if let Ok(mut g) = self.inner.lock() {
            let buf = g.out.entry(id.to_string()).or_default();
            if buf.len() < SINK_CAP {
                buf.extend_from_slice(data);
            }
        }
        self.cv.notify_all();
    }
    fn state(&self, id: &str, state: &SessionState) {
        if let Ok(mut g) = self.inner.lock() {
            if *state != SessionState::Running {
                g.ended.insert(id.to_string(), state.clone());
            }
        }
        self.cv.notify_all();
    }
}

impl DiagSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn text(&self, id: &str) -> String {
        self.inner
            .lock()
            .ok()
            .and_then(|g| {
                g.out
                    .get(id)
                    .map(|b| String::from_utf8_lossy(b).into_owned())
            })
            .unwrap_or_default()
    }

    pub fn wait_text(&self, id: &str, needle: &str, timeout: Duration) -> bool {
        let end = Instant::now() + timeout;
        let Ok(mut g) = self.inner.lock() else {
            return false;
        };
        loop {
            if g.out
                .get(id)
                .is_some_and(|b| String::from_utf8_lossy(b).contains(needle))
            {
                return true;
            }
            let now = Instant::now();
            if now >= end {
                return false;
            }
            g = match self.cv.wait_timeout(g, end - now) {
                Ok((g, _)) => g,
                Err(_) => return false,
            };
        }
    }

    pub fn ended(&self, id: &str) -> Option<SessionState> {
        self.inner
            .lock()
            .ok()
            .and_then(|g| g.ended.get(id).cloned())
    }
}

/// Everything a run needs from the outside. Built by the Tauri layer for a real run and by tests
/// (with fakes, in [`Mode::Simulated`]) for the rest.
pub struct DiagEnv<'a> {
    pub mode: Mode,
    pub report: &'a WslReport,
    pub runner: &'a Arc<dyn CommandRunner>,
    pub programs: &'a Programs,
    /// A terminal manager used only by diagnostics, plus the sink it reports to.
    pub terminal: Option<(&'a TerminalManager, &'a DiagSink)>,
    /// Host path of the distribution's file share (`\\wsl.localhost\<name>`); `None` = unavailable.
    pub linux_root: Option<&'a Path>,
    pub ops: Option<&'a dyn DistroOps>,
    pub resolver: &'a dyn KnownFolderResolver,
    pub drives: &'a dyn DriveInspector,
    pub mapping_store: &'a MappingStore,
    pub storage: &'a StorageConfig,
    /// Fresh detection (used to verify a start).
    pub detect: &'a dyn Fn() -> WslReport,
    pub windows_dir_exists: &'a dyn Fn(&str) -> bool,
    /// Windows Explorer's path, or `None` when this host has none.
    pub explorer: Option<PathBuf>,
    /// A Windows folder that always exists, in forward-slash form (`C:/Windows`), for `wslpath`.
    pub windows_probe_dir: Option<String>,
    pub now: u64,
    /// Names the temporary folder; `vlx-` + 8..24 lower-case alphanumerics.
    pub token: String,
    pub budget: Duration,
    /// How long to wait for each terminal step.
    pub step_timeout: Duration,
    /// Names to scrub from the copyable text (Windows user, distribution user, ...).
    pub scrub: Vec<String>,
}

/// What one check concluded.
struct Res {
    outcome: Outcome,
    message: String,
    guidance: Option<String>,
    modified: bool,
}

/// Read `VLXID:<user>:<home>:END` out of terminal output. The echoed command line also contains
/// `VLXID:` but with format placeholders, so only an answer without `%` or `$` counts.
fn parse_identity(text: &str) -> Option<(String, String)> {
    text.lines().rev().find_map(|line| {
        let rest = line.split("VLXID:").nth(1)?;
        if rest.contains('%') || rest.contains('$') {
            return None;
        }
        let body = rest.trim_end().strip_suffix(":END")?;
        let (user, home) = body.split_once(':')?;
        Some((user.to_string(), home.to_string()))
    })
}

fn pass(m: impl Into<String>) -> Res {
    Res {
        outcome: Outcome::Passed,
        message: m.into(),
        guidance: None,
        modified: false,
    }
}
fn fail(m: impl Into<String>, g: impl Into<String>) -> Res {
    Res {
        outcome: Outcome::Failed,
        message: m.into(),
        guidance: Some(g.into()),
        modified: false,
    }
}
fn skip(m: impl Into<String>) -> Res {
    Res {
        outcome: Outcome::Skipped,
        message: m.into(),
        guidance: None,
        modified: false,
    }
}
fn unsupported(m: impl Into<String>) -> Res {
    Res {
        outcome: Outcome::Unsupported,
        message: m.into(),
        guidance: None,
        modified: false,
    }
}
fn not_tested(m: impl Into<String>) -> Res {
    Res {
        outcome: Outcome::NotTested,
        message: m.into(),
        guidance: None,
        modified: false,
    }
}
impl Res {
    fn modified(mut self) -> Self {
        self.modified = true;
        self
    }
}

struct Spec {
    id: &'static str,
    category: Category,
    title: &'static str,
    checked: &'static str,
    /// Needs `allow_write`.
    writes: bool,
    /// Runs on the Windows host only (no WSL involved).
    host_only: bool,
}

fn spec(id: &'static str, category: Category, title: &'static str, checked: &'static str) -> Spec {
    Spec {
        id,
        category,
        title,
        checked,
        writes: false,
        host_only: false,
    }
}

fn write_spec(
    id: &'static str,
    category: Category,
    title: &'static str,
    checked: &'static str,
) -> Spec {
    Spec {
        writes: true,
        ..spec(id, category, title, checked)
    }
}

fn host_spec(
    id: &'static str,
    category: Category,
    title: &'static str,
    checked: &'static str,
) -> Spec {
    Spec {
        host_only: true,
        ..spec(id, category, title, checked)
    }
}

struct Run<'a, 'p> {
    env: &'a DiagEnv<'a>,
    opts: &'a Options,
    cancel: &'a CancelToken,
    progress: &'p mut dyn FnMut(&CheckResult),
    started: Instant,
    results: Vec<CheckResult>,
    distro: Option<String>,
    wsl_ready: bool,
    running: bool,
    share_ok: bool,
    spaces: Option<Spaces>,
    home: Option<String>,
    mount_root: String,
    automount: bool,
    diag_dir: Option<String>,
    session: Option<String>,
    /// User names discovered while running (the terminal's `id -un`); they are scrubbed from the
    /// copyable report even when the caller did not know them in advance.
    learned: Vec<String>,
}

const DIAG_PREFIX: &str = "vlinux-diag-";
/// The hidden folder `fs.home_roundtrip` creates (and removes) inside the user's home.
const HOME_TEST_PREFIX: &str = ".vlinux-validate-";

impl<'a, 'p> Run<'a, 'p> {
    fn wants(&self, c: Category) -> bool {
        self.opts.categories.as_ref().is_none_or(|v| v.contains(&c))
    }

    fn check(&mut self, sp: Spec, f: impl FnOnce(&mut Self) -> Res) {
        let t0 = Instant::now();
        let gate = if self.cancel.is_cancelled() {
            Some(not_tested("The run was cancelled before this check."))
        } else if self.started.elapsed() > self.env.budget {
            Some(not_tested(
                "The time budget for this run was used up before this check.",
            ))
        } else if sp.writes && !self.opts.allow_write {
            Some(not_tested("Needs your permission to create files in a temporary folder inside the distribution (not given for this run)."))
        } else {
            None
        };
        let v = match gate {
            Some(v) => v,
            None => f(self),
        };
        // A check that did not actually run (precondition, permission, unsupported) carries no evidence.
        let executed = !matches!(
            v.outcome,
            Outcome::NotTested | Outcome::Skipped | Outcome::Unsupported
        );
        let evidence = if !executed {
            Evidence::None
        } else if self.env.mode == Mode::Simulated {
            Evidence::Simulated
        } else if sp.host_only {
            Evidence::Host
        } else {
            Evidence::RealWsl
        };
        let r = CheckResult {
            id: sp.id.to_string(),
            category: sp.category,
            title: sp.title.to_string(),
            checked: sp.checked.to_string(),
            distro: if sp.host_only {
                None
            } else {
                self.distro.clone()
            },
            outcome: v.outcome,
            evidence,
            modified: executed && v.modified,
            message: v.message,
            guidance: v.guidance,
            ms: t0.elapsed().as_millis() as u64,
        };
        (self.progress)(&r);
        self.results.push(r);
    }

    fn passed(&self, id: &str) -> bool {
        self.results
            .iter()
            .any(|r| r.id == id && r.outcome == Outcome::Passed)
    }

    /// Common gate for checks that need the distribution to be running.
    fn need_running(&self) -> Option<Res> {
        if self.distro.is_none() {
            return Some(not_tested("No distribution was selected for this run."));
        }
        if !self.wsl_ready {
            return Some(skip("WSL is not ready (see the Runtime checks)."));
        }
        if !self.passed("runtime.distro_state") {
            return Some(skip(
                "The selected distribution is not usable (see the Runtime checks).",
            ));
        }
        if !self.running {
            return Some(not_tested(
                "The distribution is stopped, and starting it was not allowed for this run.",
            ));
        }
        None
    }

    fn need_share(&self) -> Option<Res> {
        if let Some(v) = self.need_running() {
            return Some(v);
        }
        if !self.share_ok || self.spaces.is_none() {
            return Some(skip(
                "The distribution's file share is not reachable (see the Filesystem checks).",
            ));
        }
        None
    }

    fn need_dir(&self) -> Option<Res> {
        if let Some(v) = self.need_share() {
            return Some(v);
        }
        if self.diag_dir.is_none() {
            return Some(skip(
                "The temporary diagnostic folder could not be created.",
            ));
        }
        None
    }

    fn dir(&self) -> String {
        self.diag_dir.clone().unwrap_or_default()
    }

    fn spaces(&self) -> Option<&Spaces> {
        self.spaces.as_ref()
    }

    fn loc(&self, rel: &str) -> Location {
        Location::linux(&format!("{}/{}", self.dir(), rel))
    }

    // ---- runtime -----------------------------------------------------------------------------

    fn runtime(&mut self) {
        let report = self.env.report;
        self.check(
            spec(
                "runtime.wsl_available",
                Category::Runtime,
                "WSL is available",
                "Whether wsl.exe answered and reports a usable WSL.",
            ),
            |_| match &report.status {
                RuntimeStatus::Ready | RuntimeStatus::NoDistributions => pass("WSL answered."),
                RuntimeStatus::UnsupportedHost { reason } => unsupported(reason.clone()),
                RuntimeStatus::NotInstalled => fail(
                    "WSL is not installed.",
                    "Open Linux Setup to install it (it asks for your approval first).",
                ),
                RuntimeStatus::RestartRequired => fail(
                    "Windows needs a restart before WSL can be used.",
                    "Restart Windows, then run the diagnostics again.",
                ),
                RuntimeStatus::FeatureProblem { hint, .. } => fail(
                    "A Windows feature WSL needs is missing or disabled.",
                    hint.clone(),
                ),
                RuntimeStatus::Error { message } => fail(
                    format!("WSL did not respond sensibly: {message}"),
                    "Run `wsl --status` in a terminal to see what it says.",
                ),
            },
        );
        let answered = matches!(
            report.status,
            RuntimeStatus::Ready | RuntimeStatus::NoDistributions
        );
        self.wsl_ready = report.status == RuntimeStatus::Ready;

        self.check(spec("runtime.wsl_version", Category::Runtime, "WSL version", "Whether `wsl --version` reports the WSL and kernel versions."), |_| {
            if !answered {
                return skip("WSL is not available.");
            }
            match &report.wsl_version {
                Some(v) => pass(format!("WSL {v}{}", report.kernel_version.as_ref().map(|k| format!(", kernel {k}")).unwrap_or_default())),
                None => unsupported("This WSL does not report a version (an older inbox WSL). Update WSL with `wsl --update` for full support."),
            }
        });

        self.check(
            spec(
                "runtime.distros_listed",
                Category::Runtime,
                "Distributions listed",
                "Whether WSL lists at least one distribution that VLINUX may use.",
            ),
            |_| {
                if !answered {
                    return skip("WSL is not available.");
                }
                let n = report
                    .distros
                    .iter()
                    .filter(|d| d.kind == DistroKind::Regular)
                    .count();
                if n > 0 {
                    pass(format!("{n} usable distribution(s) listed."))
                } else {
                    fail(
                        "No usable distribution is registered.",
                        "Open Linux Setup to install one.",
                    )
                }
            },
        );

        let want = self.opts.distro.clone();
        self.distro = want.clone();
        self.check(
            spec(
                "runtime.distro_state",
                Category::Runtime,
                "Selected distribution",
                "Whether the selected distribution is registered, usable, and what state it is in.",
            ),
            |s| {
                let Some(name) = &want else {
                    return not_tested("No distribution was selected for this run.");
                };
                if !s.wsl_ready {
                    return skip("WSL is not ready.");
                }
                match check_target(report, name) {
                    Ok(d) => {
                        s.running = d.state == DistroState::Running;
                        pass(format!(
                            "'{name}' is {}.",
                            if s.running { "running" } else { "stopped" }
                        ))
                    }
                    Err(e) => fail(
                        e.to_string(),
                        "Choose another distribution, or wait for WSL to finish.",
                    ),
                }
            },
        );

        let usable = self.passed("runtime.distro_state");
        let sp = spec(
            "runtime.distro_start",
            Category::Runtime,
            "Distribution starts",
            "Whether WSL can start the distribution and it is then listed as running.",
        );
        if !usable {
            self.check(sp, |_| skip("The distribution was not usable (see above)."));
        } else if self.running {
            self.check(sp, |_| {
                skip("Already running, so starting it was not exercised.")
            });
        } else {
            let allow_start = self.opts.allow_start;
            let wsl = WslClient::new(self.env.runner.clone(), self.env.programs.wsl.clone());
            let name = want.clone().unwrap_or_default();
            self.check(sp, |s| {
                if !allow_start {
                    return not_tested("The distribution is stopped; starting it needs your permission (not given for this run).");
                }
                match wsl.start(&name) {
                    Err(e) => fail(format!("WSL could not start '{name}': {e}."), "Run `wsl -d <name>` in a terminal to see the error, or check Linux Setup."),
                    Ok(o) if !o.success() => fail(
                        format!("WSL could not start '{name}' (exit code {}).", o.status.map(|c| c.to_string()).unwrap_or_else(|| "unknown".into())),
                        "Run `wsl -d <name>` in a terminal to see the error.",
                    ),
                    Ok(_) => {
                        let after = (s.env.detect)();
                        let up = after.distros.iter().any(|d| d.name == name && d.state == DistroState::Running);
                        s.running = up;
                        if up {
                            pass("Started and listed as running.").modified()
                        } else {
                            fail("WSL reported success but the distribution is not listed as running.", "It may have gone idle again; try the diagnostics once more.").modified()
                        }
                    }
                }
            });
        }

        let token = self.env.token.clone();
        let runner = self.env.runner.clone();
        let program = self.env.programs.wsl.clone();
        let cancel = self.cancel;
        self.check(
            spec("runtime.exec", Category::Runtime, "Commands run inside the distribution", "A fixed read-only command (`printf`) is run through wsl.exe and its output must come back exactly."),
            |s| {
                if let Some(v) = s.need_running() {
                    return v;
                }
                if !valid_token(&token) {
                    return fail("Internal error: bad diagnostic token.", "Report this.");
                }
                let name = s.distro.clone().unwrap_or_default();
                let p = match Prober::new(&runner, &program, &name, cancel) {
                    Ok(p) => p,
                    Err(e) => return fail(e, "Choose a valid distribution."),
                };
                match p.echo_token(&token) {
                    Ok(Probed::Value(v)) if v == token => pass("A command ran inside the distribution and returned the expected text."),
                    Ok(Probed::Value(v)) => fail(format!("The command ran but returned unexpected text ({} characters).", v.len()), "Something in the distribution's setup is altering command output."),
                    Ok(Probed::Absent) => fail("The command did not succeed.", "Run `wsl -d <name> -- echo hi` in a terminal to see why."),
                    Err(e) => fail(format!("The command could not run: {e}."), "Check that the distribution starts (`wsl -d <name>`)."),
                }
            },
        );
    }

    // ---- terminal ----------------------------------------------------------------------------

    fn terminal(&mut self) {
        let cat = Category::Terminal;
        let target = self.distro.clone().map(|name| Target {
            label: name.clone(),
            name,
        });
        self.check(
            write_spec("terminal.pty_create", cat, "Terminal session opens", "A throw-away pseudo-terminal session is opened in the distribution (as its default user, never root)."),
            |s| {
                if let Some(v) = s.need_running() {
                    return v;
                }
                let Some((mgr, _)) = s.env.terminal else { return not_tested("No terminal backend was provided for this run.") };
                let Some(t) = &target else { return not_tested("No distribution was selected.") };
                match mgr.open_target(t, s.env.programs, PtySize::DEFAULT) {
                    Ok(info) => {
                        s.session = Some(info.id);
                        pass("A session opened.").modified()
                    }
                    Err(e) => fail(e.to_string(), "Open a terminal from the Terminal app to see the same error in context."),
                }
            },
        );

        fn step(s: &mut Run<'_, '_>, send: &str, expect: &str, what: &str, hint: &str) -> Res {
            let Some(id) = s.session.clone() else {
                return skip("No terminal session is open.");
            };
            let Some((mgr, sink)) = s.env.terminal else {
                return not_tested("No terminal backend.");
            };
            if let Err(e) = mgr.write(&id, send.as_bytes()) {
                return fail(format!("Could not send input: {e}"), hint);
            }
            if sink.wait_text(&id, expect, s.env.step_timeout) {
                pass(what).modified()
            } else {
                fail(
                    format!(
                        "The expected output did not appear within {} ms.",
                        s.env.step_timeout.as_millis()
                    ),
                    hint,
                )
            }
        }

        self.check(
            write_spec(
                "terminal.io",
                cat,
                "Input and output",
                "Typed input reaches the shell and its output comes back (`printf 'VLX%sEND' 42`).",
            ),
            |s| {
                step(
                    s,
                    "printf 'VLX%sEND\\n' 42\r",
                    "VLX42END",
                    "Input reached the shell and output came back.",
                    "The shell may be slow to start or has an unusual prompt; try again.",
                )
            },
        );
        self.check(
            write_spec("terminal.identity", cat, "Runs as your normal user", "The terminal's shell reports its user and HOME (`id -un`, `$HOME`); it must not be root and HOME must be set. The user name is not shown or put in the report."),
            |s| {
                let Some(id) = s.session.clone() else { return skip("No terminal session is open.") };
                let Some((mgr, sink)) = s.env.terminal else { return not_tested("No terminal backend.") };
                if let Err(e) = mgr.write(&id, "printf 'VLXID:%s:%s:END\\n' \"$(id -un)\" \"$HOME\"\r".as_bytes()) {
                    return fail(format!("Could not send input: {e}"), "Try again.");
                }
                if !sink.wait_text(&id, ":END\r\n", s.env.step_timeout) {
                    return fail(format!("The shell did not answer within {} ms.", s.env.step_timeout.as_millis()), "The shell may be slow to start; try again.");
                }
                let ident = parse_identity(&sink.text(&id));
                if let Some((user, _)) = &ident {
                    s.learned.push(user.clone());
                }
                match ident {
                    None => fail("The answer could not be read.", "Report this."),
                    Some((user, _)) if user == "root" => fail("The terminal runs as root.", "VLINUX opens your distribution's default user. Check `[user] default=` in /etc/wsl.conf (VLINUX will not change it)."),
                    Some((_, home)) if home.is_empty() || !home.starts_with('/') => fail("HOME is not set to an absolute folder.", "Check the distribution's login configuration."),
                    Some(_) => pass("The shell runs as a non-root user with HOME set.").modified(),
                }
            },
        );
        self.check(
            write_spec(
                "terminal.ansi",
                cat,
                "Colour and control sequences",
                "ANSI escape sequences pass through unchanged (a red text sequence is printed).",
            ),
            |s| {
                step(
                    s,
                    "printf '\\033[31mVLXRED\\033[0m\\n'\r",
                    "\u{1b}[31mVLXRED",
                    "ANSI sequences arrived intact.",
                    "Something between the shell and the terminal is rewriting escape sequences.",
                )
            },
        );
        self.check(
            write_spec(
                "terminal.unicode",
                cat,
                "Unicode",
                "A non-ASCII character (a check mark) survives the round trip.",
            ),
            |s| {
                step(
                    s,
                    "printf '\\342\\234\\223VLXU\\n'\r",
                    "\u{2713}VLXU",
                    "A non-ASCII character arrived intact.",
                    "Check the distribution's locale (UTF-8) and the terminal encoding.",
                )
            },
        );
        self.check(write_spec("terminal.resize", cat, "Resize", "Resizing the terminal to 41 rows by 123 columns is seen by the shell (`stty size`)."), |s| {
            let Some(id) = s.session.clone() else { return skip("No terminal session is open.") };
            let Some((mgr, _)) = s.env.terminal else { return not_tested("No terminal backend.") };
            if let Err(e) = mgr.resize(&id, 41, 123) {
                return fail(format!("Resize failed: {e}"), "Report this.");
            }
            step(s, "stty size\r", "41 123", "The shell saw the new size.", "The pseudo-terminal did not pass the new size on.")
        });
        self.check(
            write_spec(
                "terminal.close",
                cat,
                "Session closes",
                "The session ends and no process of the diagnostics terminal is left.",
            ),
            |s| {
                let Some(id) = s.session.take() else {
                    return skip("No terminal session is open.");
                };
                let Some((mgr, _)) = s.env.terminal else {
                    return not_tested("No terminal backend.");
                };
                match mgr.close(&id) {
                    Ok(()) if mgr.live_count() == 0 => {
                        pass("The session closed and nothing was left running.")
                    }
                    Ok(()) => fail(
                        "The session closed but the manager still reports a live session.",
                        "Report this.",
                    ),
                    Err(e) => fail(format!("Closing failed: {e}"), "Report this."),
                }
            },
        );
        // Whatever happened above, never leave a diagnostics session behind.
        if let (Some(id), Some((mgr, _))) = (self.session.take(), self.env.terminal) {
            let _ = mgr.close(&id);
        }
    }

    // ---- filesystem --------------------------------------------------------------------------

    /// `full == false` runs only what the other categories need (the share and the folder).
    fn filesystem(&mut self, full: bool) {
        let cat = Category::Filesystem;
        let root = self.env.linux_root.map(|p| p.to_path_buf());
        self.check(
            spec("fs.share_access", cat, "File share reachable", "Whether the distribution's files can be reached from Windows (\\\\wsl.localhost\\<name>) and the root listed."),
            |s| {
                if let Some(v) = s.need_running() {
                    return v;
                }
                let Some(root) = &root else {
                    return if cfg!(windows) {
                        fail(
                            "The distribution's file share (\\\\wsl.localhost\\<name>) could not be found.",
                            "Make sure the distribution is running and WSL is up to date (`wsl --update`).",
                        )
                    } else {
                        unsupported("Linux files are reached through WSL's file share, which exists only on Windows.")
                    };
                };
                match fs::read_dir(root) {
                    Ok(it) => {
                        let n = it.take(10_000).count();
                        let home = s.env.ops.and_then(|o| o.home_dir().ok());
                        let (mount_root, automount) = fs::read_to_string(root.join("etc").join("wsl.conf"))
                            .map(|t| parse_wsl_conf(&t))
                            .map(|c| (c.mount_root, c.automount_enabled))
                            .unwrap_or(("/mnt".to_string(), true));
                        s.share_ok = true;
                        s.mount_root = mount_root.clone();
                        s.automount = automount;
                        s.home = home.clone();
                        s.spaces = Some(Spaces { linux_root: Some(root.clone()), linux_home: home, mount_root, windows: Vec::new(), mapped_links: Vec::new() });
                        pass(format!("The share is reachable ({n} entries at the root)."))
                    }
                    Err(e) => fail(format!("The share could not be read: {e}"), "Make sure the distribution can start and that WSL is up to date (`wsl --update`)."),
                }
            },
        );

        if full {
            self.check(spec("fs.enumerate", cat, "Folder listing", "Listing /etc through the same code the Files app uses."), |s| {
                if let Some(v) = s.need_share() {
                    return v;
                }
                let Some(spaces) = s.spaces() else { return skip("No file share.") };
                let store = ListStore::new();
                match store.open_dir(spaces, &Location::linux("/etc"), &ListOptions { show_hidden: true, ..Default::default() }, 200) {
                    Ok(head) => {
                        store.close(&head.id);
                        if head.total > 0 {
                            pass(format!("{} entries listed.", head.total))
                        } else {
                            fail("/etc listed as empty.", "A normal distribution has files in /etc; the file share may be returning incomplete results.")
                        }
                    }
                    Err(e) => fail(e.message, "See the notes in the Files app."),
                }
            });
        }

        let name = format!(
            "{DIAG_PREFIX}{}",
            self.env.token.strip_prefix("vlx-").unwrap_or("x")
        );
        self.check(
            write_spec("fs.create_dir", cat, "Create a folder", "Creates a temporary folder /tmp/vlinux-diag-<token> (all other write checks happen inside it)."),
            |s| {
                if let Some(v) = s.need_share() {
                    return v;
                }
                if !valid_token(&s.env.token) {
                    return fail("Internal error: bad diagnostic token.", "Report this.");
                }
                let Some(spaces) = s.spaces() else { return skip("No file share.") };
                match ops::create_dir(spaces, &Location::linux("/tmp"), &name) {
                    Ok(_) => {
                        s.diag_dir = Some(format!("/tmp/{name}"));
                        pass(format!("Created /tmp/{name}.")).modified()
                    }
                    Err(e) => fail(e.message, "The default user may not be able to write to /tmp; check the distribution."),
                }
            },
        );

        if !full {
            return;
        }

        self.check(
            write_spec(
                "fs.create_file",
                cat,
                "Create a file",
                "Creates an empty file in the diagnostic folder.",
            ),
            |s| {
                if let Some(v) = s.need_dir() {
                    return v;
                }
                let Some(spaces) = s.spaces() else {
                    return skip("No file share.");
                };
                match ops::create_file(spaces, &Location::linux(&s.dir()), "a.txt") {
                    Ok(_) => pass("Created a.txt.").modified(),
                    Err(e) => fail(e.message, "File creation through the file share failed."),
                }
            },
        );

        self.check(
            write_spec(
                "fs.read_file",
                cat,
                "Write and read a file",
                "Writes a short text to the new file and reads it back unchanged.",
            ),
            |s| {
                if let Some(v) = s.need_dir() {
                    return v;
                }
                if !s.passed("fs.create_file") {
                    return skip("The file could not be created.");
                }
                let Some(spaces) = s.spaces() else {
                    return skip("No file share.");
                };
                let entry = match spaces.resolve_entry(&s.loc("a.txt")) {
                    Ok(e) => e,
                    Err(e) => return fail(e.message, "Could not resolve the test file."),
                };
                let text = "vlinux diagnostics \u{2713}";
                if let Err(e) = fs::write(&entry.host, text) {
                    return fail(
                        format!("Writing failed: {e}"),
                        "The file share rejected the write.",
                    );
                }
                match fs::read_to_string(&entry.host) {
                    Ok(t) if t == text => pass("The text read back unchanged.").modified(),
                    Ok(_) => fail(
                        "The text read back differently from what was written.",
                        "The file share is altering file content (encoding or line endings).",
                    )
                    .modified(),
                    Err(e) => fail(
                        format!("Reading failed: {e}"),
                        "The file share rejected the read.",
                    )
                    .modified(),
                }
            },
        );

        self.check(
            write_spec(
                "fs.rename",
                cat,
                "Rename",
                "Renames a.txt to b.txt and checks the old name is gone.",
            ),
            |s| {
                if let Some(v) = s.need_dir() {
                    return v;
                }
                if !s.passed("fs.create_file") {
                    return skip("The file could not be created.");
                }
                let Some(spaces) = s.spaces() else {
                    return skip("No file share.");
                };
                match ops::rename(spaces, &s.loc("a.txt"), "b.txt") {
                    Ok(_) => {
                        let names = s.names_in("");
                        if names.contains(&"b.txt".to_string())
                            && !names.contains(&"a.txt".to_string())
                        {
                            pass("Renamed; the old name is gone.").modified()
                        } else {
                            fail(
                                "The rename reported success but the listing disagrees.",
                                "The file share may be caching directory contents.",
                            )
                            .modified()
                        }
                    }
                    Err(e) => fail(e.message, "Renaming through the file share failed."),
                }
            },
        );

        self.check(
            write_spec(
                "fs.copy",
                cat,
                "Copy",
                "Copies b.txt into a sub-folder and compares the content.",
            ),
            |s| {
                if let Some(v) = s.need_dir() {
                    return v;
                }
                if !s.passed("fs.rename") {
                    return skip("The file was not renamed.");
                }
                let Some(spaces) = s.spaces() else {
                    return skip("No file share.");
                };
                if let Err(e) = ops::create_dir(spaces, &Location::linux(&s.dir()), "sub") {
                    return fail(e.message, "Could not create the sub-folder.");
                }
                if let Err(m) = s.run_transfer(&s.loc("b.txt"), &s.loc("sub"), TransferMode::Copy) {
                    return fail(m, "Copying through the file share failed.").modified();
                }
                let (a, b) = (s.read_rel("sub/b.txt"), s.read_rel("b.txt"));
                if a.is_some() && a == b {
                    pass("The copy exists and matches the original.").modified()
                } else {
                    fail(
                        "The copy is missing or differs from the original.",
                        "The file share did not copy the content correctly.",
                    )
                    .modified()
                }
            },
        );

        self.check(
            write_spec(
                "fs.move",
                cat,
                "Move",
                "Moves the copy into another sub-folder and checks it left the first one.",
            ),
            |s| {
                if let Some(v) = s.need_dir() {
                    return v;
                }
                if !s.passed("fs.copy") {
                    return skip("The copy did not work.");
                }
                let Some(spaces) = s.spaces() else {
                    return skip("No file share.");
                };
                if let Err(e) = ops::create_dir(spaces, &Location::linux(&s.dir()), "sub2") {
                    return fail(e.message, "Could not create the second sub-folder.");
                }
                if let Err(m) =
                    s.run_transfer(&s.loc("sub/b.txt"), &s.loc("sub2"), TransferMode::Move)
                {
                    return fail(m, "Moving through the file share failed.").modified();
                }
                if s.read_rel("sub2/b.txt").is_some() && s.read_rel("sub/b.txt").is_none() {
                    pass("The file moved; it is gone from the source.").modified()
                } else {
                    fail(
                        "After the move the file is not only in the destination.",
                        "The file share did not move the file correctly.",
                    )
                    .modified()
                }
            },
        );

        self.check(
            write_spec("fs.symlink", cat, "Symbolic link detection", "Creates a link inside the distribution (`ln -s`) and checks the file layer recognises it as a link and reports its target."),
            |s| {
                if let Some(v) = s.need_dir() {
                    return v;
                }
                let Some(dops) = s.env.ops else { return not_tested("No way to run commands in the distribution was provided.") };
                let Some(spaces) = s.spaces() else { return skip("No file share.") };
                if let Err(e) = ops::create_dir(spaces, &Location::linux(&s.dir()), "target") {
                    return fail(e.message, "Could not create the link target folder.");
                }
                if let Err(e) = ops::create_file(spaces, &s.loc("target"), "inside.txt") {
                    return fail(e.message, "Could not create a file in the target folder.");
                }
                let target = format!("{}/target", s.dir());
                let link = format!("{}/link-to-target", s.dir());
                if let Err(e) = dops.make_symlink(&target, &link) {
                    return fail(format!("Creating the link failed: {e}"), "`ln -s` failed inside the distribution.").modified();
                }
                let found = s.entries_in("").into_iter().find(|e| e.name == "link-to-target");
                match found {
                    Some(e) if e.kind == EntryKind::Symlink => {
                        let note = match &e.link_target {
                            Some(t) if t.trim_end_matches('/') == target => "and its target matches".to_string(),
                            Some(_) => "the file share reports its target in its own form (VLINUX does not depend on that text)".to_string(),
                            None => "no target text was reported (VLINUX does not depend on it)".to_string(),
                        };
                        pass(format!("The link was recognised as a symbolic link; {note}.")).modified()
                    }
                    Some(e) => fail(format!("The link was listed as {:?}, not as a symbolic link.", e.kind), "The file share presents links differently; the Files app may not show them as links.").modified(),
                    None => fail("The link was created but is not listed.", "The file share may not show links at all.").modified(),
                }
            },
        );

        self.check(
            write_spec("fs.symlink_no_follow", cat, "Deleting a link keeps its target", "Deletes the link through the file layer and checks the folder it pointed to is untouched."),
            |s| {
                if let Some(v) = s.need_dir() {
                    return v;
                }
                if !s.passed("fs.symlink") {
                    return skip("No link was created.");
                }
                let Some(spaces) = s.spaces() else { return skip("No file share.") };
                let mut cb = |_: &Progress| {};
                let mut r = Runner::new(s.cancel, &mut cb);
                let res = ops::delete(spaces, &[s.loc("link-to-target")], true, &mut r);
                let rep = r.finish();
                if let Err(e) = res {
                    return fail(e.message, "Deleting the link failed.");
                }
                if !rep.failed.is_empty() {
                    return fail("Deleting the link reported errors.", "Check permissions in /tmp.").modified();
                }
                let names = s.names_in("");
                let inside = s.names_in("target");
                if !names.contains(&"link-to-target".to_string()) && inside.contains(&"inside.txt".to_string()) {
                    pass("The link is gone and the folder it pointed to still has its file.").modified()
                } else {
                    fail("Either the link remains or the link target lost its content.", "Do not use Delete on links until this is resolved, and report it.").modified()
                }
            },
        );

        self.check(
            spec("fs.permission_error", cat, "Permission errors are reported", "Listing /root as the default user must be refused and reported as 'permission denied' (no file content is read)."),
            |s| {
                if let Some(v) = s.need_share() {
                    return v;
                }
                let Some(spaces) = s.spaces() else { return skip("No file share.") };
                let store = ListStore::new();
                match store.open_dir(spaces, &Location::linux("/root"), &ListOptions::default(), 5) {
                    Err(e) if e.kind == FsErrorKind::PermissionDenied => pass("Access to /root was refused and reported as a permission error."),
                    Err(e) if e.kind == FsErrorKind::NotFound => skip("/root does not exist in this distribution."),
                    Err(e) => fail(format!("Access was refused but not as a permission error: {}", e.message), "Permission problems may be shown with the wrong explanation."),
                    Ok(head) => {
                        store.close(&head.id);
                        skip("The default user can read /root (it may be root), so the permission path was not exercised.")
                    }
                }
            },
        );

        // ---- the real home folder (M6.1) ----------------------------------------------------
        self.check(
            spec("fs.home_listing", cat, "Home folder listing", "Lists your Linux home folder through the same code the Files app uses (read-only). Only counts are reported - never file names."),
            |s| {
                if let Some(v) = s.need_share() {
                    return v;
                }
                let Some(home) = s.home.clone() else {
                    return fail("The home folder of the default user could not be determined.", "Check that the distribution starts and has a default user.");
                };
                let Some(spaces) = s.spaces() else { return skip("No file share.") };
                let store = ListStore::new();
                match store.open_dir(spaces, &Location::linux(&home), &ListOptions { show_hidden: true, ..Default::default() }, 500) {
                    Ok(head) => {
                        store.close(&head.id);
                        let count = |k: EntryKind| head.page.entries.iter().filter(|e| e.kind == k).count();
                        pass(format!(
                            "Listed {} entries ({} folders, {} files, {} symbolic links{}).",
                            head.total,
                            count(EntryKind::Dir),
                            count(EntryKind::File),
                            count(EntryKind::Symlink),
                            if (head.total as usize) > head.page.entries.len() { "; counts cover the first page only" } else { "" }
                        ))
                    }
                    Err(e) if e.kind == FsErrorKind::PermissionDenied => fail("Your home folder was refused as a permission error.", "The default user should own its home folder; check the distribution's permissions."),
                    Err(e) => fail(e.message, "See the notes in the Files app."),
                }
            },
        );

        self.check(
            write_spec("fs.home_roundtrip", cat, "Create, rename, copy, move and delete in your home", "Inside ONE new hidden folder in your home (`.vlinux-validate-<token>`) creates a test file, renames it, copies it, moves the copy, then deletes that folder and nothing else. No existing file or folder is read, changed or removed."),
            |s| {
                if let Some(v) = s.need_share() {
                    return v;
                }
                if !valid_token(&s.env.token) {
                    return fail("Internal error: bad diagnostic token.", "Report this.");
                }
                let Some(home) = s.home.clone() else {
                    return fail("The home folder of the default user could not be determined.", "Check that the distribution starts and has a default user.");
                };
                if home == "/" || !home.starts_with('/') || home.ends_with('/') {
                    return skip("The home folder is not an ordinary folder, so no test files were created in it.");
                }
                s.home_roundtrip(&home)
            },
        );
    }

    /// The steps of `fs.home_roundtrip`. Everything happens inside `<home>/.vlinux-validate-<token>`,
    /// which this run creates and removes; it is removed even when a step fails.
    fn home_roundtrip(&self, home: &str) -> Res {
        let token = self.env.token.strip_prefix("vlx-").unwrap_or("x").to_string();
        let name = format!("{HOME_TEST_PREFIX}{token}");
        let dir = format!("{home}/{name}");
        let Some(spaces) = self.spaces.clone() else { return skip("No file share.") };
        let loc = |rel: &str| Location::linux(&format!("{dir}/{rel}"));
        if let Err(e) = ops::create_dir(&spaces, &Location::linux(home), &name) {
            return fail(e.message, "The default user could not create a folder in its home; nothing was changed.");
        }
        let steps = (|| -> Result<(), String> {
            ops::create_file(&spaces, &Location::linux(&dir), "t1.txt").map_err(|e| format!("create: {}", e.message))?;
            let host = spaces.resolve_entry(&loc("t1.txt")).map_err(|e| format!("create: {}", e.message))?.host;
            let text = "vlinux home check \u{2713}";
            fs::write(&host, text).map_err(|e| format!("write: {e}"))?;
            if fs::read_to_string(&host).ok().as_deref() != Some(text) {
                return Err("write: the text read back differently".into());
            }
            ops::rename(&spaces, &loc("t1.txt"), "t2.txt").map_err(|e| format!("rename: {}", e.message))?;
            let names = self.names_at(&spaces, &dir);
            if !names.contains(&"t2.txt".to_string()) || names.contains(&"t1.txt".to_string()) {
                return Err("rename: the listing disagrees with the result".into());
            }
            ops::create_dir(&spaces, &Location::linux(&dir), "a").map_err(|e| format!("copy: {}", e.message))?;
            ops::create_dir(&spaces, &Location::linux(&dir), "b").map_err(|e| format!("move: {}", e.message))?;
            self.run_transfer(&loc("t2.txt"), &loc("a"), TransferMode::Copy).map_err(|m| format!("copy: {m}"))?;
            let copied = spaces.resolve_entry(&loc("a/t2.txt")).ok().and_then(|e| fs::read_to_string(e.host).ok());
            if copied.as_deref() != Some(text) {
                return Err("copy: the copy is missing or differs".into());
            }
            self.run_transfer(&loc("a/t2.txt"), &loc("b"), TransferMode::Move).map_err(|m| format!("move: {m}"))?;
            let moved = spaces.resolve_entry(&loc("b/t2.txt")).ok().and_then(|e| fs::read_to_string(e.host).ok());
            let left = spaces.resolve_entry(&loc("a/t2.txt")).ok().and_then(|e| fs::symlink_metadata(e.host).ok());
            if moved.as_deref() != Some(text) || left.is_some() {
                return Err("move: the file is not only in the destination".into());
            }
            Ok(())
        })();
        // Remove exactly the folder this run created (checked against the expected path).
        let cleaned = if dir == format!("{home}/{HOME_TEST_PREFIX}{token}") {
            let never = CancelToken::new();
            let mut cb = |_: &Progress| {};
            let mut r = Runner::new(&never, &mut cb);
            let res = ops::delete(&spaces, &[Location::linux(&dir)], true, &mut r);
            let rep = r.finish();
            res.is_ok() && rep.failed.is_empty() && self.names_at(&spaces, home).iter().all(|n| n != &name)
        } else {
            false
        };
        let remove_hint = format!("Remove the folder named {name} from your home folder yourself (it holds only the test files).");
        match (steps, cleaned) {
            (Ok(()), true) => pass("Created a folder, a file, renamed, copied and moved it, and removed that folder. Nothing else was touched.").modified(),
            (Ok(()), false) => fail("The test worked but its folder could not be removed completely.", remove_hint).modified(),
            (Err(m), true) => fail(format!("A step failed ({m}). The test folder was removed."), "Open the Files app to see the same error in context.").modified(),
            (Err(m), false) => fail(format!("A step failed ({m}) and the test folder could not be removed."), remove_hint).modified(),
        }
    }

    fn names_at(&self, spaces: &Spaces, path: &str) -> Vec<String> {
        let store = ListStore::new();
        match store.open_dir(spaces, &Location::linux(path), &ListOptions { show_hidden: true, ..Default::default() }, 1000) {
            Ok(head) => {
                store.close(&head.id);
                head.page.entries.into_iter().map(|e| e.name).collect()
            }
            Err(_) => Vec::new(),
        }
    }

    fn entries_in(&self, rel: &str) -> Vec<crate::fs::Entry> {
        let Some(spaces) = self.spaces() else {
            return Vec::new();
        };
        let path = if rel.is_empty() {
            self.dir()
        } else {
            format!("{}/{}", self.dir(), rel)
        };
        let store = ListStore::new();
        match store.open_dir(
            spaces,
            &Location::linux(&path),
            &ListOptions {
                show_hidden: true,
                ..Default::default()
            },
            500,
        ) {
            Ok(head) => {
                store.close(&head.id);
                head.page.entries
            }
            Err(_) => Vec::new(),
        }
    }

    fn names_in(&self, rel: &str) -> Vec<String> {
        self.entries_in(rel).into_iter().map(|e| e.name).collect()
    }

    fn read_rel(&self, rel: &str) -> Option<String> {
        let spaces = self.spaces()?;
        let e = spaces.resolve_entry(&self.loc(rel)).ok()?;
        fs::read_to_string(&e.host).ok()
    }

    fn run_transfer(
        &self,
        src: &Location,
        dest: &Location,
        mode: TransferMode,
    ) -> Result<(), String> {
        let Some(spaces) = self.spaces() else {
            return Err("No file share.".into());
        };
        let mut cb = |_: &Progress| {};
        let mut r = Runner::new(self.cancel, &mut cb);
        let res = ops::transfer(
            spaces,
            std::slice::from_ref(src),
            dest,
            mode,
            Conflict::Fail,
            &mut r,
        );
        let rep = r.finish();
        res.map_err(|e| e.message)?;
        if let Some(f) = rep.failed.first() {
            return Err(format!("{}: {}", f.path, f.message));
        }
        Ok(())
    }

    fn cleanup(&mut self) {
        let Some(dir) = self.diag_dir.clone() else {
            return;
        };
        // Runs even after a cancellation: it removes only what this run itself created.
        let token = self
            .env
            .token
            .strip_prefix("vlx-")
            .unwrap_or("x")
            .to_string();
        let expect = format!("/tmp/{DIAG_PREFIX}{token}");
        let t0 = Instant::now();
        let v = if dir != expect {
            fail(
                "Refused to clean up: the folder is not the one this run created.",
                "Nothing was removed.",
            )
        } else if let Some(spaces) = self.spaces.as_ref() {
            let never = CancelToken::new();
            let mut cb = |_: &Progress| {};
            let mut r = Runner::new(&never, &mut cb);
            let res = ops::delete(spaces, &[Location::linux(&dir)], true, &mut r);
            let rep = r.finish();
            let gone = self
                .env
                .linux_root
                .map(|root| {
                    let mut p = root.to_path_buf();
                    for c in linux_components(&dir).unwrap_or_default() {
                        p.push(c);
                    }
                    matches!(fs::symlink_metadata(&p), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
                })
                .unwrap_or(false);
            match res {
                Ok(()) if rep.failed.is_empty() && gone => pass(format!(
                    "Removed {dir} and everything the run created in it."
                ))
                .modified(),
                Ok(()) => fail(
                    format!("{dir} could not be removed completely."),
                    format!("Remove {dir} yourself (in a terminal: rm -r {dir})."),
                )
                .modified(),
                Err(e) => fail(
                    e.message,
                    format!("Remove {dir} yourself (in a terminal: rm -r {dir})."),
                ),
            }
        } else {
            fail(
                "No file share to clean up through.",
                format!("Remove {dir} yourself."),
            )
        };
        let r = CheckResult {
            id: "fs.cleanup".into(),
            category: Category::Filesystem,
            title: "Clean-up".into(),
            checked: "Removes only the temporary diagnostic folder this run created.".into(),
            distro: self.distro.clone(),
            outcome: v.outcome,
            evidence: if self.env.mode == Mode::Simulated {
                Evidence::Simulated
            } else {
                Evidence::RealWsl
            },
            modified: v.modified,
            message: v.message,
            guidance: v.guidance,
            ms: t0.elapsed().as_millis() as u64,
        };
        if r.outcome == Outcome::Passed {
            self.diag_dir = None;
        }
        (self.progress)(&r);
        self.results.push(r);
    }

    // ---- shared folders ----------------------------------------------------------------------

    fn shared(&mut self) {
        let cat = Category::SharedFolders;
        let resolver = self.env.resolver;
        self.check(host_spec("shared.known_folders", cat, "Windows Known Folders", "Whether Windows reports the locations of Downloads, Documents, Pictures, Music, Videos and Desktop."), |_| {
            let found: Vec<&str> = KnownFolder::ALL.iter().filter(|f| resolver.resolve(**f).is_some()).map(|f| f.label()).collect();
            if found.is_empty() {
                fail("Windows did not report any Known Folder.", "Shared folders cannot be used on this PC until Windows reports them.")
            } else {
                pass(format!("{} of {} resolved ({}).", found.len(), KnownFolder::ALL.len(), found.join(", ")))
            }
        });

        let root = self.env.linux_root.map(|p| p.to_path_buf());
        self.check(
            spec(
                "shared.mount_root",
                cat,
                "Drive mount root",
                "Reads /etc/wsl.conf for the drive mount root and whether automount is enabled.",
            ),
            |s| {
                if let Some(v) = s.need_share() {
                    return v;
                }
                let Some(root) = &root else {
                    return unsupported("No file share.");
                };
                let (present, conf) = match fs::read_to_string(root.join("etc").join("wsl.conf")) {
                    Ok(t) => (true, parse_wsl_conf(&t)),
                    Err(_) => (false, Default::default()),
                };
                pass(format!(
                    "Mount root {}, automount {}{}.",
                    conf.mount_root,
                    if conf.automount_enabled { "on" } else { "off" },
                    if present {
                        ""
                    } else {
                        " (no /etc/wsl.conf: WSL defaults)"
                    }
                ))
            },
        );

        let root = self.env.linux_root.map(|p| p.to_path_buf());
        self.check(spec("shared.mount_access", cat, "Drive mounts reachable", "Lists the drive mount root (for example /mnt) to see that Windows drives are mounted."), |s| {
            if let Some(v) = s.need_share() {
                return v;
            }
            if !s.automount {
                return skip("Automount is disabled in this distribution, so no drives are mounted.");
            }
            let Some(root) = &root else { return unsupported("No file share.") };
            let mut p = root.clone();
            for c in linux_components(&s.mount_root).unwrap_or_default() {
                p.push(c);
            }
            match fs::read_dir(&p) {
                Ok(it) => {
                    let n = it.take(1000).count();
                    if n > 0 {
                        pass(format!("{n} mount(s) under {}.", s.mount_root))
                    } else {
                        fail(format!("{} is empty: no Windows drive is mounted.", s.mount_root), "Check [automount] in /etc/wsl.conf and restart the distribution.")
                    }
                }
                Err(e) => fail(format!("{} could not be read: {e}", s.mount_root), "Check [automount] in /etc/wsl.conf and restart the distribution."),
            }
        });

        self.check(
            write_spec(
                "shared.link_roundtrip",
                cat,
                "Link create / verify / reverse",
                "Creates a link with the same operations the mapping feature uses, checks it points where intended, removes it, and checks the target still exists. Uses a folder inside the diagnostic folder, not your real shared folders.",
            ),
            |s| {
                if let Some(v) = s.need_dir() {
                    return v;
                }
                let Some(dops) = s.env.ops else { return not_tested("No way to run commands in the distribution was provided.") };
                let Some(spaces) = s.spaces() else { return skip("No file share.") };
                if let Err(e) = ops::create_dir(spaces, &Location::linux(&s.dir()), "share-target") {
                    return fail(e.message, "Could not create the link target.");
                }
                let target = format!("{}/share-target", s.dir());
                let link = format!("{}/share-link", s.dir());
                if let Err(e) = dops.make_symlink(&target, &link) {
                    return fail(format!("Creating the link failed: {e}"), "`ln -s` failed inside the distribution.").modified();
                }
                let mut problem: Option<Res> = None;
                match dops.read_link(&link) {
                    Ok(Some(t)) if engine::same_path(&t, &target) => {}
                    Ok(Some(t)) => problem = Some(fail(format!("The link points to '{t}', not where it was created to point."), "Reading links back through the distribution is unreliable here.").modified()),
                    Ok(None) => problem = Some(fail("The link was created but reads back as not a link.", "Links may not be supported on this distribution's file system.").modified()),
                    Err(e) => problem = Some(fail(format!("Reading the link failed: {e}"), "`readlink` failed inside the distribution.").modified()),
                }
                if let Err(e) = dops.remove_symlink(&link) {
                    return fail(format!("Removing the link failed: {e}"), format!("Remove {link} yourself (`unlink {link}`).")).modified();
                }
                if let Some(p) = problem {
                    return p;
                }
                match (dops.read_link(&link), dops.dir_exists(&target)) {
                    (Ok(None), Ok(true)) => pass("The link was created, verified and removed; its target was left in place.").modified(),
                    (Ok(Some(_)), _) => fail("The link is still there after removal.", "`unlink` did not remove it.").modified(),
                    (_, Ok(false)) => fail("Removing the link also removed its target.", "Do not use the mapping Undo until this is resolved, and report it.").modified(),
                    (Err(e), _) | (_, Err(e)) => fail(format!("Verifying the result failed: {e}"), "Report this.").modified(),
                }
            },
        );

        self.check(
            spec("shared.conflict_detection", cat, "Mapping state evaluation", "Evaluates each known folder's current state (read-only) the way Settings does: ready, linked, or in conflict."),
            |s| {
                if let Some(v) = s.need_share() {
                    return v;
                }
                let (Some(dops), Some(root), Some(home)) = (s.env.ops, s.env.linux_root, s.home.clone()) else {
                    return not_tested("The home folder or the means to read links was not available.");
                };
                let ctx = Ctx {
                    distro: s.distro.as_deref().unwrap_or(""),
                    linux_root: root,
                    home: &home,
                    mount_root: &s.mount_root,
                    automount: s.automount,
                    ops: dops,
                    drives: s.env.drives,
                    resolver: s.env.resolver,
                    windows_dir_exists: s.env.windows_dir_exists,
                    now: s.env.now,
                };
                let mut counts: HashMap<&'static str, usize> = HashMap::new();
                for f in KnownFolder::ALL {
                    let st = engine::status(&ctx, s.env.mapping_store, f, s.env.storage.folder(f), true);
                    let k = match st.state {
                        MappingState::Unknown => "unknown",
                        MappingState::Private => "private",
                        MappingState::Ready { .. } => "ready",
                        MappingState::Applied { .. } => "linked",
                        MappingState::Conflict { .. } => "in conflict (reported, nothing changed)",
                        MappingState::LinkedButPrivate { .. } => "linked but set private",
                        MappingState::Unavailable { .. } => "unavailable",
                        MappingState::Failed { .. } => "unreadable",
                    };
                    *counts.entry(k).or_default() += 1;
                }
                if counts.contains_key("unreadable") {
                    return fail("The state of some folders could not be read.", "Check that the distribution is reachable and try again.");
                }
                let mut parts: Vec<String> = counts.into_iter().map(|(k, n)| format!("{n} {k}")).collect();
                parts.sort();
                pass(format!("Evaluated {} folders without changing anything: {}.", KnownFolder::ALL.len(), parts.join(", ")))
            },
        );
    }

    // ---- desktop -----------------------------------------------------------------------------

    fn desktop(&mut self) {
        let cat = Category::Desktop;
        let explorer = self.env.explorer.clone();
        self.check(
            host_spec(
                "desktop.explorer",
                cat,
                "Windows Explorer",
                "Whether explorer.exe exists (used by 'Show in Explorer' and 'Open').",
            ),
            |_| match &explorer {
                None => unsupported("This host has no Windows Explorer."),
                Some(p) if p.is_file() => pass("explorer.exe was found."),
                Some(_) => fail(
                    "explorer.exe was not found at its expected location.",
                    "'Show in Explorer' will not work.",
                ),
            },
        );

        let drives = self.env.drives;
        let mount = if self.mount_root.is_empty() {
            "/mnt".to_string()
        } else {
            self.mount_root.clone()
        };
        self.check(host_spec("desktop.path_logic", cat, "Windows to WSL path conversion", "Converts a sample drive path and a network (UNC) path with VLINUX's own conversion rules."), |_| {
            let a = windows_to_wsl("C:\\Users\\Example\\Documents", &mount, drives);
            let b = windows_to_wsl("\\\\server\\share\\folder", &mount, drives);
            let want = format!("{}/c/Users/Example/Documents", mount.trim_end_matches('/'));
            match (&a, &b) {
                (Verdict::Supported { wsl_path } | Verdict::Warning { wsl_path, .. }, Verdict::Unsupported { .. }) if *wsl_path == want => {
                    pass("A drive path converted correctly and a network path was refused as designed.")
                }
                _ => fail("The conversion rules gave an unexpected answer.", "Report this; shared-folder mapping depends on it."),
            }
        });

        let probe = self.env.windows_probe_dir.clone();
        let runner = self.env.runner.clone();
        let program = self.env.programs.wsl.clone();
        let cancel = self.cancel;
        self.check(spec("desktop.wslpath", cat, "wslpath agrees", "Asks the distribution (`wslpath -u`) to convert a real Windows folder and compares it with VLINUX's conversion."), |s| {
            if let Some(v) = s.need_running() {
                return v;
            }
            let Some(win) = &probe else { return unsupported("This host has no Windows folder to convert.") };
            let name = s.distro.clone().unwrap_or_default();
            let p = match Prober::new(&runner, &program, &name, cancel) {
                Ok(p) => p,
                Err(e) => return fail(e, "Choose a valid distribution."),
            };
            let mount = if s.mount_root.is_empty() { "/mnt".to_string() } else { s.mount_root.clone() };
            let drive = win.chars().next().map(|c| c.to_ascii_lowercase()).unwrap_or('c');
            let rest = win.get(2..).unwrap_or("");
            let want = format!("{}/{}{}", mount.trim_end_matches('/'), drive, rest);
            match p.wslpath(win) {
                Ok(Probed::Value(v)) if v.eq_ignore_ascii_case(&want) => pass("wslpath and VLINUX agree on the conversion."),
                Ok(Probed::Value(v)) => fail(format!("wslpath answered '{v}' but VLINUX expected '{want}'."), "Shared-folder links may point to the wrong place; check the mount root in /etc/wsl.conf."),
                Ok(Probed::Absent) => fail("wslpath did not answer.", "The distribution may lack `wslpath`, or the drive is not mounted."),
                Err(e) => fail(format!("wslpath could not run: {e}."), "Check that the distribution starts."),
            }
        });

        let runner = self.env.runner.clone();
        let program = self.env.programs.wsl.clone();
        self.check(
            write_spec(
                "desktop.unusual_names",
                cat,
                "Unusual file names",
                "Creates files whose names contain a space, accented and symbol characters, an apostrophe and `;&`, then checks they list back with exactly those names.",
            ),
            |s| {
                if let Some(v) = s.need_dir() {
                    return v;
                }
                let Some(spaces) = s.spaces() else { return skip("No file share.") };
                let names = ["with space.txt", "ünïcödé-\u{2713}.txt", "it's.txt", "semi;amp&.txt"];
                let parent = Location::linux(&s.dir());
                let failed: Vec<&str> = names.iter().copied().filter(|n| ops::create_file(spaces, &parent, n).is_err()).collect();
                if !failed.is_empty() {
                    return fail(format!("Could not create {} file(s): {}.", failed.len(), failed.join(", ")), "Files with such names may not be creatable through the file share.").modified();
                }
                let listed = s.names_in("");
                let missing: Vec<&str> = names.iter().copied().filter(|n| !listed.iter().any(|l| l.as_str() == *n)).collect();
                if !missing.is_empty() {
                    return fail(format!("{} name(s) did not list back exactly: {}.", missing.len(), missing.join(", ")), "Names with these characters may be shown wrongly in Files.").modified();
                }
                // The distribution itself must see the name with a space too.
                let mut note = String::new();
                let dname = s.distro.clone().unwrap_or_default();
                if let Ok(p) = Prober::new(&runner, &program, &dname, s.cancel) {
                    match p.test_path("-f", &format!("{}/with space.txt", s.dir())) {
                        Ok(true) => note = " The distribution also sees the file with a space.".into(),
                        Ok(false) => return fail("The file with a space is not visible from inside the distribution.", "Paths with spaces may be mishandled between Windows and Linux.").modified(),
                        Err(e) => note = format!(" (The in-distribution check could not run: {e}.)"),
                    }
                }
                pass(format!("All {} unusual names were created and listed back exactly.{note}", names.len())).modified()
            },
        );

        self.check(host_spec("desktop.file_association", cat, "File associations", "Whether Windows would open a file type with the expected application."), |_| {
            not_tested("VLINUX has no safe, read-only way to query Windows file associations yet, so this is not tested.")
        });
    }
}

/// Run diagnostics. `progress` is called after each check (and may be used to emit events).
pub fn run(
    env: &DiagEnv<'_>,
    opts: &Options,
    cancel: &CancelToken,
    progress: &mut dyn FnMut(&CheckResult),
) -> Report {
    let mut r = Run {
        env,
        opts,
        cancel,
        progress,
        started: Instant::now(),
        results: Vec::new(),
        distro: None,
        wsl_ready: false,
        running: false,
        share_ok: false,
        spaces: None,
        home: None,
        mount_root: String::new(),
        automount: true,
        diag_dir: None,
        session: None,
        learned: Vec::new(),
    };
    // Runtime always runs: every other category depends on what it finds.
    r.runtime();
    if r.wants(Category::Terminal) {
        r.terminal();
    }
    // Shared folders and Desktop need the share and the temporary folder even when Filesystem was
    // not asked for; in that case only those two supporting checks run.
    let fs_full = r.wants(Category::Filesystem);
    if fs_full || r.wants(Category::SharedFolders) || r.wants(Category::Desktop) {
        r.filesystem(fs_full);
    }
    if r.wants(Category::SharedFolders) {
        r.shared();
    }
    if r.wants(Category::Desktop) {
        r.desktop();
    }
    r.cleanup();

    let mut results = std::mem::take(&mut r.results);
    // Stable, so clean-up stays last within Filesystem.
    results.sort_by_key(|x| x.category);
    let cancelled = cancel.is_cancelled();
    let summary = summarise(&results);
    let mut report = Report {
        generated_at: env.now,
        mode: env.mode,
        distro: opts.distro.clone(),
        allow_write: opts.allow_write,
        allow_start: opts.allow_start,
        results,
        summary,
        text: String::new(),
        cancelled,
        diag_dir: r.diag_dir.clone(),
    };
    let mut scrub = env.scrub.clone();
    scrub.extend(r.learned.iter().cloned());
    report.text = render_text(&report, &scrub);
    report
}

pub fn summarise(results: &[CheckResult]) -> Summary {
    let count = |o: Outcome| results.iter().filter(|r| r.outcome == o).count();
    Summary {
        passed: count(Outcome::Passed),
        failed: count(Outcome::Failed),
        skipped: count(Outcome::Skipped),
        unsupported: count(Outcome::Unsupported),
        not_tested: count(Outcome::NotTested),
        real_wsl_passed: results
            .iter()
            .filter(|r| r.outcome == Outcome::Passed && r.evidence == Evidence::RealWsl)
            .count(),
    }
}

fn outcome_word(o: Outcome) -> &'static str {
    match o {
        Outcome::Passed => "PASSED",
        Outcome::Failed => "FAILED",
        Outcome::Skipped => "SKIPPED",
        Outcome::Unsupported => "UNSUPPORTED",
        Outcome::NotTested => "NOT TESTED",
    }
}

fn evidence_word(e: Evidence) -> &'static str {
    match e {
        Evidence::RealWsl => "real WSL",
        Evidence::Host => "Windows host only",
        Evidence::Simulated => "SIMULATED",
        Evidence::None => "did not run",
    }
}

fn yes_no(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}

/// The copyable report. Redacted: see [`redact`].
pub fn render_text(report: &Report, scrub: &[String]) -> String {
    let mut t = String::new();
    t.push_str("VLINUX compatibility report\n");
    match report.mode {
        Mode::Real => t.push_str("Run type: REAL — checks ran against the WSL on this PC.\n"),
        Mode::Simulated => t.push_str("Run type: SIMULATED — no real WSL was contacted. These results say NOTHING about this PC.\n"),
    }
    t.push_str(&format!("Generated (unix time): {}\n", report.generated_at));
    t.push_str(&format!(
        "Distribution: {}\n",
        report.distro.as_deref().unwrap_or("(none selected)")
    ));
    t.push_str(&format!(
        "Permissions for this run: start distribution = {}, create temporary files = {}\n",
        yes_no(report.allow_start),
        yes_no(report.allow_write)
    ));
    let s = &report.summary;
    t.push_str(&format!(
        "Summary: {} passed, {} failed, {} skipped, {} unsupported, {} not tested ({} passed against the real WSL)\n",
        s.passed, s.failed, s.skipped, s.unsupported, s.not_tested, s.real_wsl_passed
    ));
    if report.cancelled {
        t.push_str("The run was cancelled; the remaining checks are marked NOT TESTED.\n");
    }
    for cat in Category::ALL {
        let rows: Vec<&CheckResult> = report
            .results
            .iter()
            .filter(|r| r.category == cat)
            .collect();
        if rows.is_empty() {
            continue;
        }
        t.push_str(&format!("\n[{}]\n", cat.title()));
        for r in rows {
            t.push_str(&format!(
                "{:<11} {} — {} [{}{}]\n",
                outcome_word(r.outcome),
                r.title,
                r.message,
                evidence_word(r.evidence),
                if r.modified {
                    ", modified temporary files"
                } else {
                    ""
                }
            ));
            if let Some(g) = &r.guidance {
                if r.outcome == Outcome::Failed {
                    t.push_str(&format!("            What to do: {g}\n"));
                }
            }
        }
    }
    t.push_str("\nNothing outside the temporary folder /tmp/vlinux-diag-* was created, changed or deleted by this run. File contents were not read except for files the run created.\n");
    redact(&t, scrub)
}

/// Remove user names from paths (`C:\Users\<name>`, `/mnt/c/Users/<name>`, `/home/<name>`) and any
/// explicit `scrub` strings (3+ characters).
pub fn redact(text: &str, scrub: &[String]) -> String {
    let mut out = text.to_string();
    for marker in ["\\Users\\", "/Users/", "/home/"] {
        let mut result = String::with_capacity(out.len());
        let mut rest = out.as_str();
        while let Some(i) = rest.find(marker) {
            let (head, tail) = rest.split_at(i + marker.len());
            result.push_str(head);
            // Windows user names may contain spaces, so there the name runs to the next separator.
            let spaces_end_name = marker != "\\Users\\";
            let end = tail
                .find(|c: char| {
                    c == '\\'
                        || c == '/'
                        || c == '\n'
                        || c == '\r'
                        || (spaces_end_name && c.is_whitespace())
                        || matches!(c, '\'' | '"' | ',' | ')' | ']' | ';')
                })
                .unwrap_or(tail.len());
            if end > 0 {
                result.push_str("<user>");
            }
            rest = &tail[end..];
        }
        result.push_str(rest);
        out = result;
    }
    for s in scrub {
        if s.chars().count() >= 3 {
            out = replace_ci(&out, s, "<user>");
        }
    }
    out
}

fn replace_ci(text: &str, needle: &str, with: &str) -> String {
    let lt = text.to_lowercase();
    let ln = needle.to_lowercase();
    // Only safe when lower-casing keeps byte offsets (ASCII); otherwise do an exact replace.
    if lt.len() != text.len() || ln.len() != needle.len() {
        return text.replace(needle, with);
    }
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while let Some(pos) = lt[i..].find(&ln) {
        out.push_str(&text[i..i + pos]);
        out.push_str(with);
        i += pos + ln.len();
    }
    out.push_str(&text[i..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::testutil::TempDir;
    use crate::model::DistroKind::{Internal, Regular};
    use crate::model::DistroState::{Running, Stopped};
    use crate::shared::testing::FakeDistro;
    use crate::storage::DriveKind;
    use crate::terminal::testing::FakeBackend;
    use crate::testing::{fake_report, out_code, out_ok, FakeRunner};
    use std::sync::atomic::{AtomicBool, Ordering};

    struct Folders;
    impl KnownFolderResolver for Folders {
        fn resolve(&self, f: KnownFolder) -> Option<String> {
            match f {
                KnownFolder::Downloads => Some("D:\\Media\\Downloads".into()),
                KnownFolder::Documents => Some("C:\\Users\\Example\\Documents".into()),
                _ => None,
            }
        }
    }
    struct NoFolders;
    impl KnownFolderResolver for NoFolders {
        fn resolve(&self, _: KnownFolder) -> Option<String> {
            None
        }
    }
    struct Fixed;
    impl DriveInspector for Fixed {
        fn kind(&self, _: char) -> DriveKind {
            DriveKind::Fixed
        }
    }

    const TOKEN: &str = "vlx-abcdef1234";
    const DIAG_FILE: &str = "/tmp/vlinux-diag-abcdef1234/with space.txt";

    /// A temporary "distribution root" with the folders a real one has.
    fn distro_root() -> TempDir {
        let d = TempDir::new("diag");
        for p in ["tmp", "etc", "home/dana", "mnt/c/Windows", "root"] {
            fs::create_dir_all(d.join(p)).unwrap();
        }
        fs::write(d.join("etc/os-release"), "ID=debian\n").unwrap();
        d
    }

    struct World {
        root: TempDir,
        _appdata: TempDir,
        report: WslReport,
        runner: Arc<FakeRunner>,
        dynr: Arc<dyn CommandRunner>,
        ops: FakeDistro,
        backend: FakeBackend,
        sink: Arc<DiagSink>,
        mgr: TerminalManager,
        store: MappingStore,
        storage: StorageConfig,
        programs: Programs,
        step: Duration,
    }

    fn default_rules() -> FakeRunner {
        FakeRunner::new()
            .on(
                "wsl.exe",
                &["-d", "Debian", "--exec", "printf", "%s", TOKEN],
                out_ok(TOKEN.as_bytes()),
            )
            .on(
                "wsl.exe",
                &["-d", "Debian", "--exec", "/bin/true"],
                out_ok(b""),
            )
            .on(
                "wsl.exe",
                &["-d", "Debian", "--exec", "wslpath", "-u", "C:/Windows"],
                out_ok(b"/mnt/c/Windows\n"),
            )
            .on(
                "wsl.exe",
                &["-d", "Debian", "--exec", "test", "-f", DIAG_FILE],
                out_ok(b""),
            )
    }

    fn world_with(state: DistroState, rules: FakeRunner) -> World {
        let root = distro_root();
        let backend = FakeBackend::new();
        let sink = Arc::new(DiagSink::new());
        let mgr = TerminalManager::new(Arc::new(backend.clone()), sink.clone());
        let appdata = TempDir::new("diag-app");
        let runner = Arc::new(rules);
        World {
            ops: FakeDistro::new(root.path(), "/home/dana"),
            report: fake_report(
                &[
                    ("Debian", state, Regular, 2),
                    ("docker-desktop", Running, Internal, 2),
                ],
                Some("Debian"),
            ),
            root,
            store: MappingStore::new(appdata.path()),
            _appdata: appdata,
            dynr: runner.clone(),
            runner,
            backend,
            sink,
            mgr,
            storage: StorageConfig::default(),
            programs: Programs::fake(),
            step: Duration::from_secs(3),
        }
    }

    fn world(state: DistroState) -> World {
        world_with(state, default_rules())
    }

    /// Answer the fake shell the way a real one would, until `stop` is set.
    fn shell_responder(backend: FakeBackend, stop: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            let Some(p) = backend.wait_proc(0, Duration::from_secs(5)) else {
                return;
            };
            let mut seen = 0usize;
            while !stop.load(Ordering::SeqCst) {
                let w = p.written_text();
                if w.len() > seen {
                    for line in w[seen..].split('\r').filter(|l| !l.is_empty()) {
                        if line.contains("VLX%sEND") {
                            p.output(b"VLX42END\r\n");
                        } else if line.contains("VLXID") {
                            p.output(b"VLXID:tester:/home/tester:END\r\n");
                        } else if line.contains("033[31m") {
                            p.output(b"\x1b[31mVLXRED\x1b[0m\r\n");
                        } else if line.contains("342") {
                            p.output("\u{2713}VLXU\r\n".as_bytes());
                        } else if line.contains("stty size") {
                            let s = p.sizes().last().copied().unwrap_or(PtySize::DEFAULT);
                            p.output(format!("{} {}\r\n", s.rows, s.cols).as_bytes());
                        }
                    }
                    seen = w.len();
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        })
    }

    fn env<'a>(
        w: &'a World,
        mode: Mode,
        report: &'a WslReport,
        detect: &'a dyn Fn() -> WslReport,
    ) -> DiagEnv<'a> {
        static EXISTS: fn(&str) -> bool = |_| true;
        DiagEnv {
            mode,
            report,
            runner: &w.dynr,
            programs: &w.programs,
            terminal: Some((&w.mgr, &*w.sink)),
            linux_root: Some(w.root.path()),
            ops: Some(&w.ops),
            resolver: &Folders,
            drives: &Fixed,
            mapping_store: &w.store,
            storage: &w.storage,
            detect,
            windows_dir_exists: &EXISTS,
            explorer: None,
            windows_probe_dir: Some("C:/Windows".into()),
            now: 1_700_000_123,
            token: TOKEN.into(),
            budget: Duration::from_secs(60),
            step_timeout: w.step,
            scrub: vec![],
        }
    }

    fn go(w: &World, mode: Mode, opts: Options, report: Option<WslReport>) -> Report {
        let report = report.unwrap_or_else(|| w.report.clone());
        let detect = || w.report.clone();
        let e = env(w, mode, &report, &detect);
        run(&e, &opts, &CancelToken::new(), &mut |_| {})
    }

    fn go_with_shell(w: &World, mode: Mode, opts: Options) -> Report {
        let stop = Arc::new(AtomicBool::new(false));
        let h = shell_responder(w.backend.clone(), stop.clone());
        let r = go(w, mode, opts, None);
        stop.store(true, Ordering::SeqCst);
        let _ = h.join();
        r
    }

    fn get<'a>(r: &'a Report, id: &str) -> &'a CheckResult {
        r.results
            .iter()
            .find(|x| x.id == id)
            .unwrap_or_else(|| panic!("no result {id}"))
    }

    fn full() -> Options {
        Options {
            distro: Some("Debian".into()),
            allow_start: false,
            allow_write: true,
            categories: None,
        }
    }

    #[test]
    fn a_simulated_run_is_never_reported_as_real_wsl() {
        let w = world(Running);
        let r = go_with_shell(&w, Mode::Simulated, full());
        assert_eq!(r.mode, Mode::Simulated);
        assert!(
            r.results
                .iter()
                .all(|x| x.evidence != Evidence::RealWsl && x.evidence != Evidence::Host),
            "{:?}",
            r.results
                .iter()
                .map(|x| (&x.id, x.evidence))
                .collect::<Vec<_>>()
        );
        assert_eq!(r.summary.real_wsl_passed, 0);
        assert!(r.text.contains("SIMULATED") && r.text.contains("NOTHING about this PC"));
        assert!(r
            .results
            .iter()
            .filter(|x| x.outcome == Outcome::Passed)
            .all(|x| x.evidence == Evidence::Simulated));
        assert!(r.summary.passed > 10);
    }

    #[test]
    fn a_real_run_labels_wsl_and_host_evidence() {
        let w = world(Running);
        let r = go(
            &w,
            Mode::Real,
            Options {
                allow_write: false,
                ..full()
            },
            None,
        );
        assert_eq!(get(&r, "runtime.wsl_available").evidence, Evidence::RealWsl);
        assert_eq!(get(&r, "shared.known_folders").evidence, Evidence::Host);
        assert_eq!(get(&r, "desktop.path_logic").evidence, Evidence::Host);
        // Not run = no evidence.
        assert_eq!(get(&r, "terminal.io").evidence, Evidence::None);
        assert!(r.text.contains("REAL"));
        assert!(r.summary.real_wsl_passed > 0);
    }

    #[test]
    fn writing_checks_need_permission_and_leave_nothing_behind() {
        let w = world(Running);
        let r = go(
            &w,
            Mode::Simulated,
            Options {
                allow_write: false,
                ..full()
            },
            None,
        );
        for id in [
            "terminal.pty_create",
            "terminal.io",
            "terminal.identity",
            "fs.create_dir",
            "fs.create_file",
            "fs.rename",
            "fs.copy",
            "fs.move",
            "fs.symlink",
            "shared.link_roundtrip",
            "desktop.unusual_names",
        ] {
            let c = get(&r, id);
            assert_eq!(c.outcome, Outcome::NotTested, "{id}");
            assert!(!c.modified);
        }
        // Read-only checks still ran.
        assert_eq!(get(&r, "fs.share_access").outcome, Outcome::Passed);
        assert_eq!(get(&r, "fs.enumerate").outcome, Outcome::Passed);
        assert!(r.results.iter().all(|x| !x.modified));
        assert_eq!(
            fs::read_dir(w.root.join("tmp")).unwrap().count(),
            0,
            "nothing may be created without permission"
        );
        assert_eq!(
            w.backend.proc_count(),
            0,
            "no terminal session without permission"
        );
        assert!(w
            .ops
            .calls()
            .iter()
            .all(|c| !c.starts_with("make_symlink") && !c.starts_with("make_dir")));
    }

    #[test]
    fn full_simulated_run_passes_and_cleans_up_only_what_it_created() {
        let w = world(Running);
        // Things the user owns, next to our folder: they must survive.
        fs::write(w.root.join("tmp/user-file.txt"), "mine").unwrap();
        fs::create_dir_all(w.root.join("tmp/other-dir")).unwrap();
        fs::write(w.root.join("tmp/other-dir/x"), "x").unwrap();
        let r = go_with_shell(&w, Mode::Simulated, full());

        for id in [
            "runtime.wsl_available",
            "runtime.wsl_version",
            "runtime.distros_listed",
            "runtime.distro_state",
            "runtime.exec",
            "terminal.pty_create",
            "terminal.io",
            "terminal.identity",
            "terminal.ansi",
            "terminal.unicode",
            "terminal.resize",
            "terminal.close",
            "fs.share_access",
            "fs.enumerate",
            "fs.home_listing",
            "fs.home_roundtrip",
            "fs.create_dir",
            "fs.create_file",
            "fs.read_file",
            "fs.rename",
            "fs.copy",
            "fs.move",
            "fs.cleanup",
            "shared.known_folders",
            "shared.mount_root",
            "shared.mount_access",
            "shared.conflict_detection",
            "desktop.path_logic",
            "desktop.wslpath",
            "desktop.unusual_names",
        ] {
            let c = get(&r, id);
            assert_eq!(c.outcome, Outcome::Passed, "{id}: {}", c.message);
        }
        if cfg!(unix) {
            for id in [
                "fs.symlink",
                "fs.symlink_no_follow",
                "shared.link_roundtrip",
            ] {
                let c = get(&r, id);
                assert_eq!(c.outcome, Outcome::Passed, "{id}: {}", c.message);
            }
        }
        assert_eq!(
            get(&r, "desktop.file_association").outcome,
            Outcome::NotTested
        );
        assert_eq!(get(&r, "desktop.explorer").outcome, Outcome::Unsupported);
        // The terminal session was closed.
        assert_eq!(w.mgr.live_count(), 0);
        // Clean-up removed exactly the diagnostic folder.
        assert!(!w.root.join("tmp/vlinux-diag-abcdef1234").exists());
        assert_eq!(
            fs::read_to_string(w.root.join("tmp/user-file.txt")).unwrap(),
            "mine"
        );
        assert_eq!(
            fs::read_to_string(w.root.join("tmp/other-dir/x")).unwrap(),
            "x"
        );
        assert!(r.diag_dir.is_none());
        // The home-folder test removed its own hidden folder and left the user's home as it was.
        let home_entries: Vec<String> = fs::read_dir(w.root.join("home/dana"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(home_entries.is_empty(), "{home_entries:?}");
        assert!(get(&r, "fs.home_roundtrip").modified);
        assert!(!get(&r, "fs.home_listing").message.contains("dana"));
        assert!(!r.text.contains("dana"), "the report must not name the user");
        assert!(get(&r, "fs.cleanup").modified);
        assert!(get(&r, "terminal.io").modified && !get(&r, "fs.enumerate").modified);
        // Results are grouped by category in order, clean-up last in its group.
        let cats: Vec<Category> = r.results.iter().map(|x| x.category).collect();
        let mut sorted = cats.clone();
        sorted.sort();
        assert_eq!(cats, sorted);
        let last_fs = r
            .results
            .iter()
            .filter(|x| x.category == Category::Filesystem)
            .last()
            .unwrap();
        assert_eq!(last_fs.id, "fs.cleanup");
    }

    #[test]
    fn failed_checks_are_classified_with_guidance() {
        let mut w = world(Running);
        w.step = Duration::from_millis(150);
        // Nothing ever answers in the terminal.
        let r = go(
            &w,
            Mode::Simulated,
            Options {
                categories: Some(vec![Category::Terminal]),
                ..full()
            },
            None,
        );
        let io = get(&r, "terminal.io");
        assert_eq!(io.outcome, Outcome::Failed);
        assert!(io.guidance.is_some());
        // The session was still closed.
        assert_eq!(w.mgr.live_count(), 0);
        assert_eq!(get(&r, "terminal.close").outcome, Outcome::Passed);
    }

    #[test]
    fn pty_failure_skips_the_dependent_steps() {
        let w = world(Running);
        w.backend.fail_next_spawn("pseudo console unavailable");
        w.backend.fail_next_spawn("pseudo console unavailable");
        let r = go(
            &w,
            Mode::Simulated,
            Options {
                categories: Some(vec![Category::Terminal]),
                ..full()
            },
            None,
        );
        assert_eq!(get(&r, "terminal.pty_create").outcome, Outcome::Failed);
        for id in [
            "terminal.io",
            "terminal.ansi",
            "terminal.unicode",
            "terminal.resize",
            "terminal.close",
        ] {
            assert_eq!(get(&r, id).outcome, Outcome::Skipped, "{id}");
        }
    }

    #[test]
    fn stopped_distro_is_not_started_without_permission() {
        let w = world(Stopped);
        let r = go(
            &w,
            Mode::Simulated,
            Options {
                allow_write: true,
                allow_start: false,
                ..full()
            },
            None,
        );
        assert_eq!(get(&r, "runtime.distro_start").outcome, Outcome::NotTested);
        assert_eq!(get(&r, "runtime.exec").outcome, Outcome::NotTested);
        assert_eq!(get(&r, "terminal.pty_create").outcome, Outcome::NotTested);
        assert_eq!(get(&r, "fs.share_access").outcome, Outcome::NotTested);
        assert!(
            !w.runner
                .call_lines()
                .iter()
                .any(|c| c.contains("/bin/true")),
            "must not start the distribution"
        );
        assert_eq!(w.backend.proc_count(), 0);
        assert_eq!(fs::read_dir(w.root.join("tmp")).unwrap().count(), 0);
    }

    #[test]
    fn stopped_distro_starts_with_permission_and_is_verified() {
        let w = world(Stopped);
        let running = fake_report(&[("Debian", Running, Regular, 2)], Some("Debian"));
        let detect = || running.clone();
        let report = w.report.clone();
        let e = env(&w, Mode::Simulated, &report, &detect);
        let r = run(
            &e,
            &Options {
                allow_write: false,
                allow_start: true,
                ..full()
            },
            &CancelToken::new(),
            &mut |_| {},
        );
        let s = get(&r, "runtime.distro_start");
        assert_eq!(s.outcome, Outcome::Passed);
        assert!(s.modified);
        assert_eq!(get(&r, "runtime.exec").outcome, Outcome::Passed);
        assert!(w
            .runner
            .call_lines()
            .iter()
            .any(|c| c.ends_with("--exec /bin/true")));
    }

    #[test]
    fn wsl_problems_skip_everything_that_depends_on_it() {
        let w = world(Running);
        let mut bad = w.report.clone();
        bad.status = RuntimeStatus::NotInstalled;
        let r = go(&w, Mode::Simulated, full(), Some(bad));
        assert_eq!(get(&r, "runtime.wsl_available").outcome, Outcome::Failed);
        assert_eq!(get(&r, "runtime.exec").outcome, Outcome::Skipped);
        assert_eq!(get(&r, "fs.share_access").outcome, Outcome::Skipped);
        assert_eq!(get(&r, "terminal.pty_create").outcome, Outcome::Skipped);
        assert_eq!(w.backend.proc_count(), 0);
        let mut unsupported_host = w.report.clone();
        unsupported_host.status = RuntimeStatus::UnsupportedHost {
            reason: "not Windows".into(),
        };
        assert_eq!(
            get(
                &go(&w, Mode::Simulated, full(), Some(unsupported_host)),
                "runtime.wsl_available"
            )
            .outcome,
            Outcome::Unsupported
        );
    }

    #[test]
    fn internal_and_unknown_distros_are_refused() {
        let w = world(Running);
        let r = go(
            &w,
            Mode::Simulated,
            Options {
                distro: Some("docker-desktop".into()),
                ..full()
            },
            None,
        );
        assert_eq!(get(&r, "runtime.distro_state").outcome, Outcome::Failed);
        assert_ne!(get(&r, "runtime.exec").outcome, Outcome::Passed);
        let r = go(
            &w,
            Mode::Simulated,
            Options {
                distro: Some("Nope".into()),
                ..full()
            },
            None,
        );
        assert_eq!(get(&r, "runtime.distro_state").outcome, Outcome::Failed);
        let r = go(
            &w,
            Mode::Simulated,
            Options {
                distro: Some("-d x; rm".into()),
                ..full()
            },
            None,
        );
        assert_eq!(get(&r, "runtime.distro_state").outcome, Outcome::Failed);
        assert_ne!(get(&r, "fs.share_access").outcome, Outcome::Passed);
        assert!(
            w.runner.calls().is_empty(),
            "nothing may run for a refused distribution"
        );
        assert_eq!(w.backend.proc_count(), 0);
        assert_eq!(fs::read_dir(w.root.join("tmp")).unwrap().count(), 0);
    }

    #[test]
    fn no_distribution_selected_runs_only_host_and_runtime_checks() {
        let w = world(Running);
        let r = go(
            &w,
            Mode::Real,
            Options {
                distro: None,
                ..full()
            },
            None,
        );
        assert_eq!(get(&r, "runtime.distro_state").outcome, Outcome::NotTested);
        assert_eq!(get(&r, "fs.share_access").outcome, Outcome::NotTested);
        assert_eq!(get(&r, "shared.known_folders").outcome, Outcome::Passed);
    }

    #[test]
    fn cancellation_marks_the_rest_not_tested() {
        let w = world(Running);
        let report = w.report.clone();
        let detect = || w.report.clone();
        let e = env(&w, Mode::Simulated, &report, &detect);
        let cancel = CancelToken::new();
        let c2 = cancel.clone();
        let mut n = 0;
        let r = run(&e, &full(), &cancel, &mut |_| {
            n += 1;
            if n == 3 {
                c2.cancel();
            }
        });
        assert!(r.cancelled);
        assert!(
            r.results
                .iter()
                .filter(|x| x.outcome == Outcome::NotTested)
                .count()
                >= 10
        );
        assert_eq!(get(&r, "runtime.wsl_available").outcome, Outcome::Passed);
        assert_eq!(get(&r, "terminal.pty_create").outcome, Outcome::NotTested);
        assert!(r.text.contains("cancelled"));
        // Even a cancelled run leaves nothing behind.
        assert_eq!(fs::read_dir(w.root.join("tmp")).unwrap().count(), 0);
    }

    #[test]
    fn cancellation_in_the_middle_still_cleans_up() {
        let w = world(Running);
        let report = w.report.clone();
        let detect = || w.report.clone();
        let e = env(&w, Mode::Simulated, &report, &detect);
        let cancel = CancelToken::new();
        let c2 = cancel.clone();
        let opts = Options {
            categories: Some(vec![Category::Filesystem]),
            ..full()
        };
        let r = run(&e, &opts, &cancel, &mut |c| {
            if c.id == "fs.rename" {
                c2.cancel();
            }
        });
        assert!(r.cancelled);
        assert_eq!(get(&r, "fs.rename").outcome, Outcome::Passed);
        assert_eq!(get(&r, "fs.copy").outcome, Outcome::NotTested);
        assert_eq!(get(&r, "fs.cleanup").outcome, Outcome::Passed);
        assert!(!w.root.join("tmp/vlinux-diag-abcdef1234").exists());
    }

    #[test]
    fn time_budget_is_enforced() {
        let w = world(Running);
        let report = w.report.clone();
        let detect = || w.report.clone();
        let mut e = env(&w, Mode::Simulated, &report, &detect);
        e.budget = Duration::from_millis(0);
        std::thread::sleep(Duration::from_millis(5));
        let r = run(&e, &full(), &CancelToken::new(), &mut |_| {});
        assert!(r
            .results
            .iter()
            .any(|x| x.outcome == Outcome::NotTested && x.message.contains("time budget")));
    }

    #[test]
    fn cleanup_refuses_a_folder_that_is_not_ours() {
        let w = world(Running);
        let report = w.report.clone();
        let detect = || w.report.clone();
        let e = env(&w, Mode::Simulated, &report, &detect);
        let cancel = CancelToken::new();
        let opts = Options::default();
        let mut noop = |_: &CheckResult| {};
        let mut run_ = Run {
            env: &e,
            opts: &opts,
            cancel: &cancel,
            progress: &mut noop,
            started: Instant::now(),
            results: Vec::new(),
            distro: Some("Debian".into()),
            wsl_ready: true,
            running: true,
            share_ok: true,
            spaces: Some(Spaces {
                linux_root: Some(w.root.path().to_path_buf()),
                linux_home: None,
                mount_root: "/mnt".into(),
                windows: vec![],
                mapped_links: vec![],
            }),
            home: None,
            mount_root: "/mnt".into(),
            automount: true,
            diag_dir: Some("/home/dana".into()),
            session: None,
            learned: Vec::new(),
        };
        run_.cleanup();
        assert_eq!(run_.results[0].outcome, Outcome::Failed);
        assert!(
            w.root.join("home/dana").exists(),
            "a folder that is not ours must never be removed"
        );
    }

    #[test]
    fn mount_checks_detect_missing_drives_and_disabled_automount() {
        let w = world(Running);
        fs::remove_dir_all(w.root.join("mnt/c")).unwrap();
        let r = go(
            &w,
            Mode::Simulated,
            Options {
                allow_write: false,
                ..full()
            },
            None,
        );
        assert_eq!(get(&r, "shared.mount_access").outcome, Outcome::Failed);
        fs::write(w.root.join("etc/wsl.conf"), "[automount]\nenabled=false\n").unwrap();
        let r = go(
            &w,
            Mode::Simulated,
            Options {
                allow_write: false,
                ..full()
            },
            None,
        );
        assert_eq!(get(&r, "shared.mount_access").outcome, Outcome::Skipped);
        assert!(get(&r, "shared.mount_root")
            .message
            .contains("automount off"));
    }

    #[test]
    fn no_known_folders_is_a_failure() {
        let w = world(Running);
        let report = w.report.clone();
        let detect = || w.report.clone();
        let mut e = env(&w, Mode::Real, &report, &detect);
        e.resolver = &NoFolders;
        let r = run(
            &e,
            &Options {
                allow_write: false,
                ..full()
            },
            &CancelToken::new(),
            &mut |_| {},
        );
        assert_eq!(get(&r, "shared.known_folders").outcome, Outcome::Failed);
    }

    #[test]
    fn exec_failures_are_reported_not_hidden() {
        let bad = FakeRunner::new().on(
            "wsl.exe",
            &["-d", "Debian", "--exec", "printf", "%s", TOKEN],
            out_code(1, b""),
        );
        let w = world_with(Running, bad);
        let r = go(
            &w,
            Mode::Simulated,
            Options {
                allow_write: false,
                categories: Some(vec![Category::Runtime]),
                ..full()
            },
            None,
        );
        assert_eq!(get(&r, "runtime.exec").outcome, Outcome::Failed);
        assert!(get(&r, "runtime.exec").guidance.is_some());
    }

    #[test]
    fn category_filter_limits_the_run() {
        let w = world(Running);
        let r = go(
            &w,
            Mode::Simulated,
            Options {
                allow_write: false,
                categories: Some(vec![Category::Runtime]),
                ..full()
            },
            None,
        );
        assert!(r.results.iter().all(|x| x.category == Category::Runtime));
        // Desktop alone still needs the supporting share/folder checks, but not the full filesystem set.
        let r = go(
            &w,
            Mode::Simulated,
            Options {
                allow_write: false,
                categories: Some(vec![Category::Desktop]),
                ..full()
            },
            None,
        );
        assert!(r.results.iter().any(|x| x.id == "fs.share_access"));
        assert!(!r.results.iter().any(|x| x.id == "fs.rename"));
    }

    #[test]
    fn report_text_is_redacted() {
        let t = redact("C:\\Users\\Dana Smith\\Documents and /mnt/c/Users/dana/x and /home/dana/.ssh and dana-pc", &["dana-pc".into()]);
        assert!(!t.contains("/home/dana") && !t.contains("/Users/dana") && !t.contains("dana-pc"));
        assert!(t.contains("C:\\Users\\<user>"));
        assert_eq!(redact("nothing here", &[]), "nothing here");
        assert_eq!(
            redact("short ab", &["ab".into()]),
            "short ab",
            "too-short scrub strings are ignored"
        );
        assert_eq!(redact("/home/", &[]), "/home/");
        assert_eq!(
            replace_ci("Dana and DANA", "dana", "<user>"),
            "<user> and <user>"
        );
    }

    #[test]
    fn report_text_contains_no_user_names() {
        let w = world(Running);
        let report = w.report.clone();
        let detect = || w.report.clone();
        let mut e = env(&w, Mode::Simulated, &report, &detect);
        e.scrub = vec!["dana".into()];
        let r = run(
            &e,
            &Options {
                allow_write: false,
                ..full()
            },
            &CancelToken::new(),
            &mut |_| {},
        );
        assert!(!r.text.contains("dana"));
        assert!(!r.text.contains("Example\\Documents"));
    }

    #[test]
    fn progress_is_reported_per_check() {
        let w = world(Running);
        let report = w.report.clone();
        let detect = || w.report.clone();
        let e = env(&w, Mode::Simulated, &report, &detect);
        let mut seen = Vec::new();
        let r = run(
            &e,
            &Options {
                allow_write: false,
                ..full()
            },
            &CancelToken::new(),
            &mut |c| seen.push(c.id.clone()),
        );
        assert_eq!(seen.len(), r.results.len());
        assert_eq!(seen[0], "runtime.wsl_available");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_target_text_differences_are_noted_not_failed() {
        // FakeDistro links point at host paths, so the file layer reports a different target text.
        let w = world(Running);
        let r = go(
            &w,
            Mode::Simulated,
            Options {
                categories: Some(vec![Category::Filesystem]),
                ..full()
            },
            None,
        );
        let c = get(&r, "fs.symlink");
        assert_eq!(c.outcome, Outcome::Passed, "{}", c.message);
        assert!(c.message.contains("symbolic link"));
    }

    #[test]
    fn evidence_never_claims_real_for_a_check_that_did_not_run() {
        let w = world(Running);
        for mode in [Mode::Real, Mode::Simulated] {
            let r = go(
                &w,
                mode,
                Options {
                    allow_write: false,
                    ..full()
                },
                None,
            );
            for c in &r.results {
                if matches!(
                    c.outcome,
                    Outcome::NotTested | Outcome::Skipped | Outcome::Unsupported
                ) {
                    assert_eq!(c.evidence, Evidence::None, "{}", c.id);
                }
            }
        }
    }

    #[test]
    fn identity_answer_is_read_only_from_a_real_answer_not_the_echo() {
        let echo =
            "printf 'VLXID:%s:%s:END\\n' \"$(id -un)\" \"$HOME\"\r\nVLXID:dana:/home/dana:END\r\n";
        assert_eq!(
            parse_identity(echo),
            Some(("dana".into(), "/home/dana".into()))
        );
        assert_eq!(
            parse_identity("printf 'VLXID:%s:%s:END\\n' \"$(id -un)\"\r\n"),
            None
        );
        assert_eq!(parse_identity("nothing here"), None);
    }

    #[test]
    fn home_checks_never_touch_existing_files_and_need_permission_to_write() {
        let w = world(Running);
        fs::write(w.root.join("home/dana/precious.txt"), "mine").unwrap();
        fs::create_dir_all(w.root.join("home/dana/Projects")).unwrap();
        fs::write(w.root.join("home/dana/Projects/x"), "x").unwrap();
        // Without permission: listing runs (read-only), the write test does not.
        let r = go(
            &w,
            Mode::Simulated,
            Options {
                allow_write: false,
                ..full()
            },
            None,
        );
        assert_eq!(get(&r, "fs.home_roundtrip").outcome, Outcome::NotTested);
        assert!(get(&r, "fs.home_listing").message.contains("2 entries"));
        assert_eq!(fs::read_dir(w.root.join("home/dana")).unwrap().count(), 2);
        // With permission: only the test folder appears and disappears.
        let r = go_with_shell(&w, Mode::Simulated, full());
        assert_eq!(get(&r, "fs.home_roundtrip").outcome, Outcome::Passed);
        assert_eq!(fs::read_dir(w.root.join("home/dana")).unwrap().count(), 2);
        assert_eq!(
            fs::read_to_string(w.root.join("home/dana/precious.txt")).unwrap(),
            "mine"
        );
        assert_eq!(
            fs::read_to_string(w.root.join("home/dana/Projects/x")).unwrap(),
            "x"
        );
    }

    #[test]
    fn a_missing_home_fails_the_roundtrip_without_creating_anything() {
        let mut w = world(Running);
        w.ops = FakeDistro::new(w.root.path(), "/home/ghost");
        let r = go(
            &w,
            Mode::Simulated,
            Options {
                categories: Some(vec![Category::Filesystem]),
                ..full()
            },
            None,
        );
        assert_eq!(get(&r, "fs.home_roundtrip").outcome, Outcome::Failed);
        assert_ne!(get(&r, "fs.home_listing").outcome, Outcome::Passed);
        assert!(!w.root.join("home/ghost").exists());
        assert_eq!(fs::read_dir(w.root.join("home")).unwrap().count(), 1);
    }
}
