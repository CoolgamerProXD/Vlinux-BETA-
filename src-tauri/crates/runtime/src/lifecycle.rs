//! Distribution lifecycle (M5): start and stop **one** distribution, on explicit request.
//!
//! What this module can never do: shut WSL down, unregister, set the Windows default, convert,
//! import, export or mount. The only `wsl.exe` verbs reachable from here are
//! [`WslClient::start`] (`-d <name> --exec /bin/true`) and [`WslClient::terminate`]
//! (`--terminate <name>`), both of which name exactly one validated, registered, non-internal
//! distribution. Stop needs an explicit `confirmed` flag.
//!
//! One operation per distribution at a time ([`OpRegistry`]); a second request is refused with
//! `Busy` instead of being run twice.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use serde::Serialize;

use crate::exec::RunError;
use crate::model::{DistroKind, DistroState, RuntimeStatus, WslReport};
use crate::parse::valid_distro_name;
use crate::wsl::{WslClient, START_TIMEOUT};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleError {
    NotReady,
    Invalid(String),
    NotFound(String),
    /// Belongs to another application (Docker Desktop, ...): never touched.
    Internal(String),
    /// Another operation on the same distribution is in progress, or WSL itself is changing it.
    Busy(String),
    /// A stop needs the user's explicit confirmation.
    NotConfirmed,
    Failed(String),
}

impl fmt::Display for LifecycleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LifecycleError::NotReady => {
                f.write_str("WSL is not ready. Check the setup status first.")
            }
            LifecycleError::Invalid(m) | LifecycleError::Failed(m) | LifecycleError::Busy(m) => {
                f.write_str(m)
            }
            LifecycleError::NotFound(n) => {
                write!(f, "No distribution named '{n}' is registered with WSL.")
            }
            LifecycleError::Internal(n) => write!(
                f,
                "'{n}' belongs to another application and is never touched by VLINUX."
            ),
            LifecycleError::NotConfirmed => {
                f.write_str("Stopping a distribution needs your confirmation. Nothing was stopped.")
            }
        }
    }
}

impl std::error::Error for LifecycleError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OpKind {
    Start,
    Stop,
    Inspect,
}

impl OpKind {
    pub fn verb(&self) -> &'static str {
        match self {
            OpKind::Start => "starting",
            OpKind::Stop => "stopping",
            OpKind::Inspect => "inspecting",
        }
    }
}

/// One in-flight operation per distribution. The token releases the slot when dropped, even if
/// the worker panics.
#[derive(Default)]
pub struct OpRegistry {
    active: Mutex<HashMap<String, OpKind>>,
}

pub struct OpToken {
    registry: Arc<OpRegistry>,
    name: String,
}

impl Drop for OpToken {
    fn drop(&mut self) {
        if let Ok(mut m) = self.registry.active.lock() {
            m.remove(&self.name);
        }
    }
}

impl OpRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn begin(self: &Arc<Self>, name: &str, kind: OpKind) -> Result<OpToken, LifecycleError> {
        let mut m = self
            .active
            .lock()
            .map_err(|_| LifecycleError::Failed("internal state is unavailable".into()))?;
        if let Some(k) = m.get(name) {
            return Err(LifecycleError::Busy(format!(
                "'{name}' is already {} — wait for that to finish.",
                k.verb()
            )));
        }
        m.insert(name.to_string(), kind);
        Ok(OpToken {
            registry: self.clone(),
            name: name.to_string(),
        })
    }

    pub fn current(&self, name: &str) -> Option<OpKind> {
        self.active.lock().ok().and_then(|m| m.get(name).copied())
    }

    pub fn snapshot(&self) -> Vec<(String, OpKind)> {
        self.active
            .lock()
            .map(|m| m.iter().map(|(k, v)| (k.clone(), *v)).collect())
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartPlan {
    AlreadyRunning,
    NeedsStart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopPlan {
    AlreadyStopped,
    Terminate,
}

/// Checks shared by every per-distribution operation.
pub fn check_target<'a>(
    report: &'a WslReport,
    name: &str,
) -> Result<&'a crate::model::Distro, LifecycleError> {
    if !valid_distro_name(name) {
        return Err(LifecycleError::Invalid(format!(
            "'{}' is not a valid distribution name.",
            name.chars().take(40).collect::<String>()
        )));
    }
    if report.status != RuntimeStatus::Ready {
        return Err(LifecycleError::NotReady);
    }
    // Exact match only: the name came from the frontend and must equal a registered name.
    let d = report
        .distros
        .iter()
        .find(|d| d.name == name)
        .ok_or_else(|| LifecycleError::NotFound(name.to_string()))?;
    if d.kind == DistroKind::Internal {
        return Err(LifecycleError::Internal(name.to_string()));
    }
    match d.state {
        DistroState::Installing | DistroState::Uninstalling | DistroState::Converting => {
            Err(LifecycleError::Busy(format!(
                "'{name}' is being changed by WSL right now. Wait for that to finish."
            )))
        }
        _ => Ok(d),
    }
}

pub fn check_start(report: &WslReport, name: &str) -> Result<StartPlan, LifecycleError> {
    let d = check_target(report, name)?;
    Ok(if d.state == DistroState::Running {
        StartPlan::AlreadyRunning
    } else {
        StartPlan::NeedsStart
    })
}

pub fn check_stop(
    report: &WslReport,
    name: &str,
    confirmed: bool,
) -> Result<StopPlan, LifecycleError> {
    let d = check_target(report, name)?;
    if !confirmed {
        return Err(LifecycleError::NotConfirmed);
    }
    Ok(if d.state == DistroState::Stopped {
        StopPlan::AlreadyStopped
    } else {
        StopPlan::Terminate
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpOutcome {
    pub distro: String,
    pub op: OpKind,
    /// Something was actually run.
    pub ran: bool,
    /// The state afterwards was confirmed by listing the distributions again.
    pub verified: bool,
    pub message: String,
}

fn stderr_line(out: &crate::exec::CmdOutput) -> String {
    let mut t = crate::encoding::decode_wsl_output(&out.stderr);
    if t.trim().is_empty() {
        t = crate::encoding::decode_wsl_output(&out.stdout);
    }
    crate::detect::first_line(&t)
}

fn run_error(name: &str, what: &str, e: &RunError) -> LifecycleError {
    match e {
        RunError::Timeout(d) => LifecycleError::Failed(format!(
            "'{name}' did not finish {what} within {} s. It may still be working: refresh in a moment to see its real state.",
            d.as_secs()
        )),
        RunError::Cancelled => LifecycleError::Failed(format!("{what} '{name}' was cancelled.")),
        other => LifecycleError::Failed(format!("Could not {what} '{name}': {other}.")),
    }
}

fn state_of(report: &WslReport, name: &str) -> Option<DistroState> {
    report
        .distros
        .iter()
        .find(|d| d.name == name)
        .map(|d| d.state)
}

/// Start one distribution. `detect` must list the distributions afresh (not from a cache).
/// Returns the outcome; the caller refreshes its cached report from `detect` as well.
pub fn start(
    wsl: &WslClient,
    report: &WslReport,
    name: &str,
    detect: &dyn Fn() -> WslReport,
) -> Result<OpOutcome, LifecycleError> {
    match check_start(report, name)? {
        StartPlan::AlreadyRunning => Ok(OpOutcome {
            distro: name.to_string(),
            op: OpKind::Start,
            ran: false,
            verified: true,
            message: format!("'{name}' is already running. Nothing was started."),
        }),
        StartPlan::NeedsStart => {
            let out = wsl
                .start(name)
                .map_err(|e| run_error(name, "starting", &e))?;
            if !out.success() {
                let detail = stderr_line(&out);
                return Err(LifecycleError::Failed(format!(
                    "WSL could not start '{name}' (exit code {}){}",
                    out.status
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "unknown".into()),
                    if detail.is_empty() {
                        String::new()
                    } else {
                        format!(": {detail}")
                    }
                )));
            }
            let after = detect();
            let running = state_of(&after, name) == Some(DistroState::Running);
            Ok(OpOutcome {
                distro: name.to_string(),
                op: OpKind::Start,
                ran: true,
                verified: running,
                message: if running {
                    format!("'{name}' is running. WSL stops an idle distribution by itself after a while; open a terminal to keep it in use.")
                } else {
                    format!("WSL reported success, but '{name}' is not listed as running (it may already have gone idle). Refresh to see its real state.")
                },
            })
        }
    }
}

/// Stop one distribution (`wsl --terminate <name>`): never the whole WSL VM, nothing deleted.
pub fn stop(
    wsl: &WslClient,
    report: &WslReport,
    name: &str,
    confirmed: bool,
    detect: &dyn Fn() -> WslReport,
) -> Result<OpOutcome, LifecycleError> {
    match check_stop(report, name, confirmed)? {
        StopPlan::AlreadyStopped => Ok(OpOutcome {
            distro: name.to_string(),
            op: OpKind::Stop,
            ran: false,
            verified: true,
            message: format!("'{name}' is already stopped. Nothing was changed."),
        }),
        StopPlan::Terminate => {
            let out = wsl
                .terminate(name)
                .map_err(|e| run_error(name, "stopping", &e))?;
            if !out.success() {
                let detail = stderr_line(&out);
                return Err(LifecycleError::Failed(format!(
                    "WSL could not stop '{name}' (exit code {}){}",
                    out.status
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "unknown".into()),
                    if detail.is_empty() {
                        String::new()
                    } else {
                        format!(": {detail}")
                    }
                )));
            }
            let after = detect();
            let stopped = state_of(&after, name) == Some(DistroState::Stopped);
            Ok(OpOutcome {
                distro: name.to_string(),
                op: OpKind::Stop,
                ran: true,
                verified: stopped,
                message: if stopped {
                    format!("'{name}' was stopped. Its files are untouched; programs that were running in it ended.")
                } else {
                    format!("WSL reported success, but '{name}' is still listed as running. Refresh to see its real state.")
                },
            })
        }
    }
}

/// The start timeout, for UI copy.
pub fn start_timeout_secs() -> u64 {
    START_TIMEOUT.as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DistroKind::{Internal, Regular};
    use crate::model::DistroState::{Converting, Running, Stopped};
    use crate::testing::{fake_report, out_code, out_ok, FakeRunner};
    use std::time::Duration;

    fn client(f: FakeRunner) -> (WslClient, Arc<FakeRunner>) {
        let r = Arc::new(f);
        (WslClient::new(r.clone(), "wsl.exe"), r)
    }

    fn base() -> WslReport {
        fake_report(
            &[
                ("Debian", Stopped, Regular, 2),
                ("Ubuntu", Running, Regular, 2),
                ("docker-desktop", Running, Internal, 2),
                ("Moving", Converting, Regular, 2),
            ],
            Some("Debian"),
        )
    }

    #[test]
    fn validation_is_exact_and_refuses_foreign_distributions() {
        let r = base();
        assert_eq!(check_start(&r, "Debian"), Ok(StartPlan::NeedsStart));
        assert_eq!(check_start(&r, "Ubuntu"), Ok(StartPlan::AlreadyRunning));
        assert!(matches!(
            check_start(&r, "debian"),
            Err(LifecycleError::NotFound(_))
        ));
        assert!(matches!(
            check_start(&r, "docker-desktop"),
            Err(LifecycleError::Internal(_))
        ));
        assert!(matches!(
            check_start(&r, "Moving"),
            Err(LifecycleError::Busy(_))
        ));
        let long = "n".repeat(70);
        for bad in ["", "-d", "a;b", "x y\n", long.as_str()] {
            assert!(
                matches!(check_start(&r, bad), Err(LifecycleError::Invalid(_))),
                "{bad:?}"
            );
        }
        let mut not_ready = base();
        not_ready.status = RuntimeStatus::NotInstalled;
        assert_eq!(
            check_start(&not_ready, "Debian"),
            Err(LifecycleError::NotReady)
        );
    }

    #[test]
    fn stop_needs_confirmation_and_never_touches_stopped_distros() {
        let r = base();
        assert_eq!(
            check_stop(&r, "Ubuntu", false),
            Err(LifecycleError::NotConfirmed)
        );
        assert_eq!(check_stop(&r, "Ubuntu", true), Ok(StopPlan::Terminate));
        assert_eq!(check_stop(&r, "Debian", true), Ok(StopPlan::AlreadyStopped));
        assert!(matches!(
            check_stop(&r, "docker-desktop", true),
            Err(LifecycleError::Internal(_))
        ));
    }

    #[test]
    fn start_runs_only_for_a_stopped_distro_and_verifies() {
        let (w, runner) = client(FakeRunner::new().on(
            "wsl.exe",
            &["-d", "Debian", "--exec", "/bin/true"],
            out_ok(b""),
        ));
        let after = fake_report(&[("Debian", Running, Regular, 2)], Some("Debian"));
        let o = start(&w, &base(), "Debian", &|| after.clone()).unwrap();
        assert!(o.ran && o.verified);
        assert_eq!(runner.call_lines(), ["wsl.exe -d Debian --exec /bin/true"]);

        // Already running: no process at all.
        let (w, runner) = client(FakeRunner::new());
        let o = start(&w, &base(), "Ubuntu", &|| unreachable!()).unwrap();
        assert!(!o.ran && o.verified);
        assert!(runner.calls().is_empty());
    }

    #[test]
    fn start_reports_unverified_failed_and_timeout_honestly() {
        let still_stopped = base();
        let (w, _) = client(FakeRunner::new().on(
            "wsl.exe",
            &["-d", "Debian", "--exec", "/bin/true"],
            out_ok(b""),
        ));
        let o = start(&w, &base(), "Debian", &|| still_stopped.clone()).unwrap();
        assert!(o.ran && !o.verified);
        assert!(o.message.contains("not listed as running"));

        let (w, _) = client(FakeRunner::new().on(
            "wsl.exe",
            &["-d", "Debian", "--exec", "/bin/true"],
            out_code(1, b""),
        ));
        let e = start(&w, &base(), "Debian", &|| unreachable!()).unwrap_err();
        assert!(matches!(e, LifecycleError::Failed(m) if m.contains("exit code 1")));

        let (w, _) = client(FakeRunner::new().on(
            "wsl.exe",
            &["-d", "Debian", "--exec", "/bin/true"],
            Err(RunError::Timeout(Duration::from_secs(60))),
        ));
        let e = start(&w, &base(), "Debian", &|| unreachable!()).unwrap_err();
        assert!(
            matches!(e, LifecycleError::Failed(m) if m.contains("60 s") && m.contains("refresh"))
        );
    }

    #[test]
    fn stop_terminates_only_the_named_distro() {
        let (w, runner) =
            client(FakeRunner::new().on("wsl.exe", &["--terminate", "Ubuntu"], out_ok(b"")));
        let after = fake_report(
            &[
                ("Debian", Stopped, Regular, 2),
                ("Ubuntu", Stopped, Regular, 2),
            ],
            None,
        );
        let o = stop(&w, &base(), "Ubuntu", true, &|| after.clone()).unwrap();
        assert!(o.ran && o.verified);
        assert_eq!(runner.call_lines(), ["wsl.exe --terminate Ubuntu"]);

        // Unconfirmed, internal and already-stopped requests never reach wsl.exe.
        let (w, runner) = client(FakeRunner::new());
        assert!(stop(&w, &base(), "Ubuntu", false, &|| unreachable!()).is_err());
        assert!(stop(&w, &base(), "docker-desktop", true, &|| unreachable!()).is_err());
        assert!(
            !stop(&w, &base(), "Debian", true, &|| unreachable!())
                .unwrap()
                .ran
        );
        assert!(runner.calls().is_empty());
    }

    #[test]
    fn stop_failure_and_unverified_are_reported() {
        let (w, _) =
            client(FakeRunner::new().on("wsl.exe", &["--terminate", "Ubuntu"], out_code(1, b"")));
        assert!(stop(&w, &base(), "Ubuntu", true, &|| unreachable!()).is_err());
        let (w, _) =
            client(FakeRunner::new().on("wsl.exe", &["--terminate", "Ubuntu"], out_ok(b"")));
        let o = stop(&w, &base(), "Ubuntu", true, &|| base()).unwrap();
        assert!(o.ran && !o.verified);
    }

    #[test]
    fn registry_dedupes_per_distro_and_releases_on_drop() {
        let reg = OpRegistry::new();
        let t = reg.begin("Debian", OpKind::Start).unwrap();
        assert!(matches!(
            reg.begin("Debian", OpKind::Start),
            Err(LifecycleError::Busy(_))
        ));
        assert!(matches!(
            reg.begin("Debian", OpKind::Stop),
            Err(LifecycleError::Busy(_))
        ));
        // A different distro is independent.
        let _u = reg.begin("Ubuntu", OpKind::Inspect).unwrap();
        assert_eq!(reg.current("Debian"), Some(OpKind::Start));
        drop(t);
        assert_eq!(reg.current("Debian"), None);
        assert!(reg.begin("Debian", OpKind::Stop).is_ok());
    }

    #[test]
    fn nothing_here_can_reach_forbidden_wsl_verbs() {
        // The client allow-list is the only door; the forbidden verbs are not on it.
        for bad in [
            &["--shutdown"][..],
            &["--unregister", "Debian"],
            &["--set-default", "Debian"],
            &["--set-version", "Debian", "2"],
            &["--import", "a", "b", "c"],
            &["--export", "a", "b"],
            &["--mount", "x"],
        ] {
            assert!(
                !crate::wsl::is_allowed_invocation(
                    &bad.iter().map(|s| s.to_string()).collect::<Vec<_>>()
                ),
                "{bad:?}"
            );
        }
    }
}
