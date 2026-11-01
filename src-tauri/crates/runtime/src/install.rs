//! Consent-gated installation.
//!
//! The flow is deliberately two-step so the UI can never run an install the user has not seen:
//!
//! 1. [`prepare_distribution_plan`] / [`prepare_wsl_platform_plan`] build a plan (what will run,
//!    what it may need, what is left untouched, and any *blockers*). Nothing runs.
//! 2. [`PlanStore::take_approved`] accepts a plan id only together with an explicit
//!    `approved == true`, re-validates against fresh detection data (so an existing distribution
//!    can never be overwritten), and hands back the plan exactly once.
//!
//! [`run_install`] then executes the one official command (`wsl.exe --install ...`), streams
//! progress, honours cancellation and classifies the outcome (success, restart required, failure,
//! cancelled, timed out). VLINUX never unregisters, resets, imports or deletes a distribution.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::adapters::AdapterInfo;
use crate::detect::probe_pending_reboot;
use crate::encoding::decode_wsl_output;
use crate::exec::{CancelToken, RunError};
use crate::model::{Catalog, DistroKind, RuntimeStatus, WslReport};
use crate::parse::{classify_failure, Failure};
use crate::wsl::{InstallTarget, Programs, WslClient};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanKind {
    Distribution,
    WslPlatform,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AdminRequirement {
    /// Nothing in this plan needs administrator rights.
    Never,
    /// Windows itself may show a UAC prompt (to enable Windows features). VLINUX does not run
    /// elevated and never requests elevation on its own.
    MayPrompt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstallPlan {
    pub plan_id: String,
    pub kind: PlanKind,
    pub wsl_name: Option<String>,
    pub display_name: String,
    pub adapter: Option<AdapterInfo>,
    pub command_preview: String,
    pub steps: Vec<String>,
    pub requires_admin: AdminRequirement,
    pub restart_possible: bool,
    pub needs_internet: bool,
    /// Distributions that exist now and will not be touched.
    pub existing_untouched: Vec<String>,
    /// Reasons this plan cannot be started. Empty = ready for approval.
    pub blockers: Vec<String>,
}

impl InstallPlan {
    pub fn target(&self) -> Option<InstallTarget> {
        match (self.kind, &self.wsl_name) {
            (PlanKind::Distribution, Some(n)) => Some(InstallTarget::Distribution(n.clone())),
            (PlanKind::WslPlatform, _) => Some(InstallTarget::WslPlatform),
            _ => None,
        }
    }
}

fn existing_names(report: &WslReport) -> Vec<String> {
    report.distros.iter().filter(|d| d.kind == DistroKind::Regular).map(|d| d.name.clone()).collect()
}

/// Why the current WSL state does not allow installing a *distribution*, if it does not.
fn distribution_blocker(status: &RuntimeStatus) -> Option<String> {
    match status {
        RuntimeStatus::Ready | RuntimeStatus::NoDistributions => None,
        RuntimeStatus::NotInstalled => Some("WSL is not installed yet. Set up WSL first.".into()),
        RuntimeStatus::RestartRequired => Some("Windows needs a restart before WSL can be used. Restart Windows, then try again.".into()),
        RuntimeStatus::FeatureProblem { hint, .. } => Some(hint.clone()),
        RuntimeStatus::Error { message } => Some(format!("WSL is not responding properly: {message}")),
        RuntimeStatus::UnsupportedHost { reason } => Some(reason.clone()),
    }
}

pub fn prepare_distribution_plan(wsl_name: &str, report: &WslReport, catalog: &Catalog) -> InstallPlan {
    let mut blockers = Vec::new();
    let entry = catalog.entries.iter().find(|e| e.wsl_name == wsl_name);
    match entry {
        None => blockers.push(match &catalog.error {
            Some(e) => format!("The official distribution list is unavailable ({e}), so VLINUX cannot verify '{wsl_name}'."),
            None => format!("'{wsl_name}' is not in Microsoft's official list of installable distributions."),
        }),
        Some(e) if !e.supported => blockers.push(format!(
            "VLINUX has no adapter for '{wsl_name}' yet, so it is not offered for installation."
        )),
        Some(_) => {}
    }
    if let Some(b) = distribution_blocker(&report.status) {
        blockers.push(b);
    }
    if report.distros.iter().any(|d| d.name.eq_ignore_ascii_case(wsl_name)) {
        blockers.push(format!(
            "A distribution named '{wsl_name}' is already installed. VLINUX never overwrites, resets or replaces an existing distribution; use it instead."
        ));
    }

    let target = InstallTarget::Distribution(wsl_name.to_string());
    let display_name = entry.map(|e| e.friendly_name.clone()).unwrap_or_else(|| wsl_name.to_string());
    InstallPlan {
        plan_id: String::new(),
        kind: PlanKind::Distribution,
        wsl_name: Some(wsl_name.to_string()),
        display_name: display_name.clone(),
        adapter: entry.and_then(|e| e.adapter.clone()),
        command_preview: WslClient::install_command_preview(&target),
        steps: vec![
            format!("Download {display_name} from Microsoft's official WSL distribution list (internet connection required)."),
            format!("Register it with WSL as '{wsl_name}', side by side with your existing distributions."),
            "Nothing is launched and no account is created in this step; first-run setup happens when the distribution is first opened.".to_string(),
        ],
        requires_admin: AdminRequirement::Never,
        restart_possible: false,
        needs_internet: true,
        existing_untouched: existing_names(report),
        blockers,
    }
}

pub fn prepare_wsl_platform_plan(report: &WslReport) -> InstallPlan {
    let mut blockers = Vec::new();
    match &report.status {
        RuntimeStatus::NotInstalled | RuntimeStatus::FeatureProblem { .. } => {}
        RuntimeStatus::RestartRequired => blockers.push("Windows already has a restart pending. Restart Windows first, then check again.".to_string()),
        RuntimeStatus::Ready | RuntimeStatus::NoDistributions => blockers.push("WSL is already installed and working.".to_string()),
        RuntimeStatus::UnsupportedHost { reason } => blockers.push(reason.clone()),
        RuntimeStatus::Error { message } => blockers.push(format!(
            "VLINUX could not tell why WSL is not working ({message}), so it will not try to reinstall it. Try 'wsl.exe --update' in a terminal or see the Windows documentation for WSL."
        )),
    }
    InstallPlan {
        plan_id: String::new(),
        kind: PlanKind::WslPlatform,
        wsl_name: None,
        display_name: "Windows Subsystem for Linux".to_string(),
        adapter: None,
        command_preview: WslClient::install_command_preview(&InstallTarget::WslPlatform),
        steps: vec![
            "Enable the Windows features WSL needs ('Windows Subsystem for Linux' and 'Virtual Machine Platform') and install the WSL kernel, using Windows' official installer.".to_string(),
            "No Linux distribution is installed in this step.".to_string(),
            "Windows may show an administrator (UAC) prompt and may ask you to restart afterwards.".to_string(),
        ],
        requires_admin: AdminRequirement::MayPrompt,
        restart_possible: true,
        needs_internet: true,
        existing_untouched: existing_names(report),
        blockers,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallError {
    NotApproved,
    UnknownPlan,
    PlanExpired,
    Blocked(Vec<String>),
    AlreadyExists(String),
    NotReady(String),
}

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InstallError::NotApproved => write!(f, "The installation was not approved."),
            InstallError::UnknownPlan => write!(f, "This installation plan is unknown or was already used. Review it again."),
            InstallError::PlanExpired => write!(f, "This installation plan has expired. Review it again."),
            InstallError::Blocked(b) => write!(f, "This installation cannot start: {}", b.join(" ")),
            InstallError::AlreadyExists(n) => write!(f, "A distribution named '{n}' already exists. VLINUX never overwrites an existing distribution."),
            InstallError::NotReady(m) => write!(f, "WSL is no longer in the state this plan was made for: {m}"),
        }
    }
}

impl std::error::Error for InstallError {}

/// Holds reviewed plans until they are approved (single use, expiring).
pub struct PlanStore {
    plans: Mutex<HashMap<String, (InstallPlan, Instant)>>,
    counter: AtomicU64,
    ttl: Duration,
}

impl Default for PlanStore {
    fn default() -> Self {
        Self::with_ttl(Duration::from_secs(15 * 60))
    }
}

impl PlanStore {
    pub fn with_ttl(ttl: Duration) -> Self {
        Self { plans: Mutex::new(HashMap::new()), counter: AtomicU64::new(1), ttl }
    }

    /// Assign an id and remember the plan for later approval.
    pub fn insert(&self, mut plan: InstallPlan) -> InstallPlan {
        let n = self.counter.fetch_add(1, Ordering::SeqCst);
        plan.plan_id = format!("plan-{n}-{:x}", crate::unix_now());
        let mut g = self.plans.lock().unwrap();
        g.retain(|_, (_, t)| t.elapsed() < self.ttl);
        g.insert(plan.plan_id.clone(), (plan.clone(), Instant::now()));
        plan
    }

    /// Consume a plan for execution. Requires explicit approval and re-validates against the
    /// current WSL state.
    pub fn take_approved(&self, plan_id: &str, approved: bool, current: &WslReport) -> Result<InstallPlan, InstallError> {
        if !approved {
            return Err(InstallError::NotApproved);
        }
        let mut g = self.plans.lock().unwrap();
        let (plan, created) = match g.get(plan_id) {
            Some((p, t)) => (p.clone(), *t),
            None => return Err(InstallError::UnknownPlan),
        };
        if created.elapsed() >= self.ttl {
            g.remove(plan_id);
            return Err(InstallError::PlanExpired);
        }
        if !plan.blockers.is_empty() {
            return Err(InstallError::Blocked(plan.blockers.clone()));
        }
        match plan.kind {
            PlanKind::Distribution => {
                if let Some(name) = &plan.wsl_name {
                    if current.distros.iter().any(|d| d.name.eq_ignore_ascii_case(name)) {
                        return Err(InstallError::AlreadyExists(name.clone()));
                    }
                }
                if let Some(b) = distribution_blocker(&current.status) {
                    return Err(InstallError::NotReady(b));
                }
            }
            PlanKind::WslPlatform => match &current.status {
                RuntimeStatus::NotInstalled | RuntimeStatus::FeatureProblem { .. } => {}
                other => return Err(InstallError::NotReady(format!("{other:?}"))),
            },
        }
        g.remove(plan_id);
        Ok(plan)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InstallEvent {
    pub job_id: String,
    /// `"progress"` or `"finished"`.
    pub kind: &'static str,
    pub message: String,
    pub percent: Option<f32>,
    pub outcome: Option<InstallOutcome>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InstallOutcome {
    Succeeded,
    /// The installer finished (or needs to finish) after a Windows restart.
    RestartRequired { message: String },
    Failed { exit_code: Option<i32>, message: String, hint: String },
    Cancelled,
    TimedOut,
}

/// Latest human message and percentage from (decoded) installer output, which uses `\r` redraws.
pub fn latest_progress(text: &str) -> (String, Option<f32>) {
    let line = text.split(['\r', '\n']).map(|l| l.trim()).rfind(|l| !l.is_empty()).unwrap_or("");
    let percent = line.rfind('%').and_then(|idx| {
        let digits: String = line[..idx]
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        digits.parse::<f32>().ok().filter(|p| (0.0..=100.0).contains(p))
    });
    let cleaned: String = line.chars().filter(|c| !matches!(*c, '[' | ']' | '=' | '#' | '>')).collect();
    let message = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    (message, percent)
}

const SUCCESS_REBOOT_REQUIRED: i32 = 3010; // ERROR_SUCCESS_REBOOT_REQUIRED

fn strict_reboot_marker(lc: &str) -> bool {
    lc.contains("0x80070bc2") || lc.contains("error_success_reboot_required")
}

fn loose_reboot_marker(lc: &str) -> bool {
    lc.contains("reboot") || lc.contains("restart")
}

fn last_message(text: &str) -> String {
    let line = text
        .split(['\r', '\n'])
        .map(|l| l.trim())
        .rfind(|l| !l.is_empty() && !l.contains('%'))
        .unwrap_or("");
    line.chars().take(240).collect()
}

const GENERIC_FAILURE_HINT: &str = "VLINUX did not change any existing distribution. Check your internet connection and try again, or run the command shown in the plan in a terminal to see WSL's full output.";

pub fn run_install(
    wsl: &WslClient,
    programs: &Programs,
    plan: &InstallPlan,
    job_id: &str,
    cancel: &CancelToken,
    emit: &mut dyn FnMut(InstallEvent),
) -> InstallOutcome {
    let Some(target) = plan.target() else {
        return InstallOutcome::Failed { exit_code: None, message: "The installation plan is incomplete.".into(), hint: GENERIC_FAILURE_HINT.into() };
    };
    let mut progress = |message: String, percent: Option<f32>| {
        emit(InstallEvent { job_id: job_id.to_string(), kind: "progress", message, percent, outcome: None });
    };
    progress(format!("Running {}", plan.command_preview), None);

    let mut buf: Vec<u8> = Vec::new();
    let mut last: (String, Option<f32>) = (String::new(), None);
    let result = wsl.install(&target, cancel, &mut |chunk| {
        buf.extend_from_slice(chunk);
        let latest = latest_progress(&decode_wsl_output(&buf));
        if latest != last && !latest.0.is_empty() {
            last = latest.clone();
            progress(latest.0, latest.1);
        }
    });

    match result {
        Err(RunError::Cancelled) => InstallOutcome::Cancelled,
        Err(RunError::Timeout(_)) => InstallOutcome::TimedOut,
        Err(RunError::NotFound(_)) => InstallOutcome::Failed {
            exit_code: None,
            message: "wsl.exe was not found on this PC.".to_string(),
            hint: "WSL is not available. Use 'Set up WSL' first.".to_string(),
        },
        Err(RunError::Io(m)) => InstallOutcome::Failed { exit_code: None, message: m, hint: GENERIC_FAILURE_HINT.to_string() },
        Ok(out) => {
            let mut text = decode_wsl_output(&out.stdout);
            let err = decode_wsl_output(&out.stderr);
            if !err.is_empty() {
                text.push('\n');
                text.push_str(&err);
            }
            let lc = text.to_lowercase();
            if out.status == Some(SUCCESS_REBOOT_REQUIRED) || (out.status != Some(0) && strict_reboot_marker(&lc)) {
                return InstallOutcome::RestartRequired { message: "Windows needs to restart to finish the installation. Save your work and restart Windows, then reopen VLINUX.".into() };
            }
            if out.status == Some(0) {
                let platform = matches!(target, InstallTarget::WslPlatform);
                if loose_reboot_marker(&lc) || (platform && probe_pending_reboot(wsl, programs)) {
                    return InstallOutcome::RestartRequired { message: "Windows needs to restart to finish the installation. Save your work and restart Windows, then reopen VLINUX.".into() };
                }
                return InstallOutcome::Succeeded;
            }
            let hint = match classify_failure(&text) {
                Some(Failure::Code { hint, .. }) => hint.to_string(),
                Some(Failure::NotInstalled) => "WSL is not installed. Use 'Set up WSL' first.".to_string(),
                _ => GENERIC_FAILURE_HINT.to_string(),
            };
            let msg = last_message(&text);
            InstallOutcome::Failed {
                exit_code: out.status,
                message: if msg.is_empty() {
                    format!("wsl.exe exited with code {}.", out.status.map(|c| c.to_string()).unwrap_or_else(|| "unknown".into()))
                } else {
                    msg
                },
                hint,
            }
        }
    }
}

/// After a failed or cancelled install: did a half-registered distribution appear? VLINUX never
/// removes it; the user is told what exists and that removal is their decision.
pub fn leftover_note(name: &str, after: &WslReport) -> Option<String> {
    after.distros.iter().find(|d| d.name.eq_ignore_ascii_case(name)).map(|d| {
        format!(
            "A distribution named '{}' is now registered with WSL (state: {:?}). VLINUX has not removed it. If it is incomplete you can remove it yourself in Windows; doing so permanently deletes that distribution's files, so VLINUX will always ask you first.",
            d.name, d.state
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_extraction() {
        assert_eq!(latest_progress("Installing: Ubuntu [=====     ] 45.5%"), ("Installing: Ubuntu 45.5%".to_string(), Some(45.5)));
        assert_eq!(latest_progress("Downloading: Ubuntu\n"), ("Downloading: Ubuntu".to_string(), None));
        assert_eq!(latest_progress("a\nInstalling 10%\nInstalling 20%\n").1, Some(20.0));
        assert_eq!(latest_progress(""), (String::new(), None));
        assert_eq!(latest_progress("100%").1, Some(100.0));
        assert_eq!(latest_progress("weird 1000%").1, None);
        // Console redraws use a bare carriage return.
        assert_eq!(latest_progress("Downloading 30%\rDownloading 60%\r"), ("Downloading 60%".to_string(), Some(60.0)));
    }

    #[test]
    fn last_message_skips_progress_lines() {
        assert_eq!(last_message("Installing 5%\nError code: Wsl/InstallDistro/Failed\nInstalling 9%"), "Error code: Wsl/InstallDistro/Failed");
    }
}
