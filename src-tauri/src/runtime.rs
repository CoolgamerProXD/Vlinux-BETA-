//! Tauri bindings for the Linux runtime layer (`vlinux-runtime`).
//!
//! Rules enforced here:
//! * Nothing runs at startup. Detection happens when the UI asks for it, is cached, and
//!   concurrent requests are coalesced.
//! * Every command is `async` and does its blocking work on a worker thread (never the UI thread).
//! * The frontend never supplies a command line. It names a *plan* (created by the backend from
//!   the official catalogue) and a distro name that is validated against the current WSL state.
//! * Installation requires an explicit `approved = true` for a reviewed, unexpired, single-use plan.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use vlinux_runtime::adapters::DistroRegistry;
use vlinux_runtime::detect::{detect, fetch_catalog, DetectOptions};
use vlinux_runtime::exec::{CancelToken, SystemRunner};
use vlinux_runtime::install::{
    leftover_note, prepare_distribution_plan, prepare_wsl_platform_plan, run_install, InstallEvent, InstallOutcome,
    InstallPlan, PlanKind, PlanStore,
};
use vlinux_runtime::managed::ManagedStore;
use vlinux_runtime::model::{Catalog, DistroKind, HostEnv, RuntimeStatus, WslReport};
use vlinux_runtime::storage::{self, DriveInspector, KnownFolderResolver, StorageOverview, SystemDrives, SystemKnownFolders, Verdict};
use vlinux_runtime::wsl::{Programs, WslClient};
use vlinux_runtime::unix_now;

use crate::config::ConfigStore;
use crate::error::AppError;

pub const INSTALL_EVENT: &str = "vlinux://install";
/// A cached detection is reused for this long unless the user presses "Check again".
const REPORT_TTL: Duration = Duration::from_secs(60);
/// A forced refresh within this window reuses the result of the one that just ran.
const FORCE_COALESCE: Duration = Duration::from_secs(2);
const CATALOG_TTL: Duration = Duration::from_secs(10 * 60);

struct ActiveJob {
    id: String,
    cancel: CancelToken,
}

pub struct RuntimeState {
    pub(crate) wsl: WslClient,
    pub(crate) programs: Programs,
    registry: DistroRegistry,
    plans: PlanStore,
    managed: ManagedStore,
    host: HostEnv,
    report: Mutex<Option<(Instant, WslReport)>>,
    catalog: Mutex<Option<(Instant, Catalog)>>,
    /// Serialises detections so simultaneous requests share one `wsl.exe` run.
    detect_gate: Mutex<()>,
    job: Mutex<Option<ActiveJob>>,
    job_counter: AtomicU64,
}

fn host_env() -> HostEnv {
    let info = os_info::get();
    let build = match info.version() {
        os_info::Version::Semantic(_, _, build) if cfg!(windows) => u32::try_from(*build).ok(),
        _ => None,
    };
    HostEnv { is_windows: cfg!(windows), windows_build: build }
}

impl RuntimeState {
    pub fn new(config_dir: &Path) -> Self {
        let programs = Programs::system();
        Self {
            wsl: WslClient::new(Arc::new(SystemRunner), programs.wsl.clone()),
            programs,
            registry: DistroRegistry::with_builtin(),
            plans: PlanStore::default(),
            managed: ManagedStore::new(config_dir),
            host: host_env(),
            report: Mutex::new(None),
            catalog: Mutex::new(None),
            detect_gate: Mutex::new(()),
            job: Mutex::new(None),
            job_counter: AtomicU64::new(1),
        }
    }

    fn cached_report(&self, max_age: Duration) -> Option<WslReport> {
        let g = self.report.lock().ok()?;
        match &*g {
            Some((at, r)) if at.elapsed() < max_age => Some(r.clone()),
            _ => None,
        }
    }

    pub(crate) fn invalidate(&self) {
        if let Ok(mut g) = self.report.lock() {
            *g = None;
        }
    }

    /// Detection with caching and coalescing. `force` = user pressed "Check again".
    pub fn detect(&self, force: bool) -> WslReport {
        let max_age = if force { FORCE_COALESCE } else { REPORT_TTL };
        if let Some(r) = self.cached_report(max_age) {
            return r;
        }
        let _gate = self.detect_gate.lock().unwrap_or_else(|p| p.into_inner());
        // Another request may have finished while we waited for the gate.
        if let Some(r) = self.cached_report(max_age) {
            return r;
        }
        let managed = self.managed.names();
        let report = detect(
            &self.wsl,
            &self.programs,
            &self.registry,
            &DetectOptions { host: self.host.clone(), managed: &managed, identify_running: true },
        );
        if let Ok(mut g) = self.report.lock() {
            *g = Some((Instant::now(), report.clone()));
        }
        report
    }

    fn catalog(&self, force: bool, report: &WslReport) -> Catalog {
        if !force {
            if let Ok(g) = self.catalog.lock() {
                if let Some((at, c)) = &*g {
                    if at.elapsed() < CATALOG_TTL && c.error.is_none() {
                        // Refresh the "already installed" flags from the current report.
                        let mut c = c.clone();
                        for e in &mut c.entries {
                            e.already_installed = report.distros.iter().any(|d| d.name.eq_ignore_ascii_case(&e.wsl_name));
                        }
                        return c;
                    }
                }
            }
        }
        let c = fetch_catalog(&self.wsl, &self.registry, &report.distros);
        if let Ok(mut g) = self.catalog.lock() {
            *g = Some((Instant::now(), c.clone()));
        }
        c
    }
}

type Rt<'a> = State<'a, Arc<RuntimeState>>;

pub(crate) async fn blocking<T, F>(f: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| AppError::Runtime(format!("background task failed: {e}")))?
}

/// Read-only snapshot of WSL and the distributions on this PC.
#[tauri::command]
pub async fn wsl_detect(rt: Rt<'_>, force: Option<bool>) -> Result<WslReport, AppError> {
    let rt = rt.inner().clone();
    blocking(move || Ok(rt.detect(force.unwrap_or(false)))).await
}

/// The official list of installable distributions (needs internet; cached).
#[tauri::command]
pub async fn wsl_catalog(rt: Rt<'_>, force: Option<bool>) -> Result<Catalog, AppError> {
    let rt = rt.inner().clone();
    blocking(move || {
        let report = rt.detect(false);
        Ok(rt.catalog(force.unwrap_or(false), &report))
    })
    .await
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanRequestKind {
    Distribution,
    WslPlatform,
}

/// Build (and remember) a reviewable plan. Nothing is installed by this call.
#[tauri::command]
pub async fn install_prepare(rt: Rt<'_>, kind: PlanRequestKind, name: Option<String>) -> Result<InstallPlan, AppError> {
    let rt = rt.inner().clone();
    blocking(move || {
        let report = rt.detect(true);
        let plan = match kind {
            PlanRequestKind::WslPlatform => prepare_wsl_platform_plan(&report),
            PlanRequestKind::Distribution => {
                let name = name.ok_or_else(|| AppError::Runtime("No distribution was chosen.".into()))?;
                let catalog = rt.catalog(false, &report);
                prepare_distribution_plan(&name, &report, &catalog)
            }
        };
        Ok(rt.plans.insert(plan))
    })
    .await
}

#[derive(Debug, Serialize)]
pub struct StartedJob {
    pub job_id: String,
}

/// Start an approved plan on a background thread; progress arrives as `vlinux://install` events.
#[tauri::command]
pub async fn install_start(app: AppHandle, rt: Rt<'_>, plan_id: String, approved: bool) -> Result<StartedJob, AppError> {
    let rt = rt.inner().clone();
    blocking(move || {
        if !approved {
            return Err(AppError::Runtime("The installation was not approved.".into()));
        }
        // Re-validate against the *current* state, not the state at review time.
        let current = rt.detect(true);
        let mut job = rt.job.lock().map_err(|_| AppError::Runtime("internal state error".into()))?;
        if job.is_some() {
            return Err(AppError::Runtime("An installation is already in progress.".into()));
        }
        let plan = rt.plans.take_approved(&plan_id, approved, &current).map_err(|e| AppError::Runtime(e.to_string()))?;
        let job_id = format!("job-{}", rt.job_counter.fetch_add(1, Ordering::SeqCst));
        let cancel = CancelToken::new();
        *job = Some(ActiveJob { id: job_id.clone(), cancel: cancel.clone() });
        drop(job);

        let worker_rt = rt.clone();
        let worker_id = job_id.clone();
        std::thread::Builder::new()
            .name("vlinux-install".into())
            .spawn(move || run_job(app, worker_rt, plan, worker_id, cancel))
            .map_err(|e| {
                if let Ok(mut j) = rt.job.lock() {
                    *j = None;
                }
                AppError::Runtime(format!("could not start the installer thread: {e}"))
            })?;
        Ok(StartedJob { job_id })
    })
    .await
}

fn run_job(app: AppHandle, rt: Arc<RuntimeState>, plan: InstallPlan, job_id: String, cancel: CancelToken) {
    let emitter = app.clone();
    let outcome = run_install(&rt.wsl, &rt.programs, &plan, &job_id, &cancel, &mut |event: InstallEvent| {
        let _ = emitter.emit(INSTALL_EVENT, event);
    });

    // Look at what exists now (fresh), so the UI shows the truth after any outcome.
    rt.invalidate();
    let after = rt.detect(true);
    let mut message = match &outcome {
        InstallOutcome::Succeeded => format!("{} was installed.", plan.display_name),
        InstallOutcome::RestartRequired { message } => message.clone(),
        InstallOutcome::Failed { message, .. } => message.clone(),
        InstallOutcome::Cancelled => "The installation was cancelled.".to_string(),
        InstallOutcome::TimedOut => "The installation took too long and was stopped.".to_string(),
    };

    if let (PlanKind::Distribution, Some(name)) = (plan.kind, plan.wsl_name.as_deref()) {
        let registered = after.distros.iter().any(|d| d.name.eq_ignore_ascii_case(name));
        match &outcome {
            InstallOutcome::Succeeded | InstallOutcome::RestartRequired { .. } if registered => {
                // The plan guaranteed this name did not exist before, so it is ours.
                if let Err(e) = rt.managed.record(name, name, unix_now()) {
                    message.push_str(&format!(" (VLINUX could not record it as installed by VLINUX: {e}.)"));
                }
                rt.invalidate();
            }
            InstallOutcome::Failed { .. } | InstallOutcome::Cancelled | InstallOutcome::TimedOut => {
                if let Some(note) = leftover_note(name, &after) {
                    message.push(' ');
                    message.push_str(&note);
                }
            }
            _ => {}
        }
    }

    if let Ok(mut j) = rt.job.lock() {
        *j = None;
    }
    let _ = app.emit(
        INSTALL_EVENT,
        InstallEvent { job_id, kind: "finished", message, percent: None, outcome: Some(outcome) },
    );
}

/// Ask the running installer to stop. Returns whether a matching job was found.
#[tauri::command]
pub async fn install_cancel(rt: Rt<'_>, job_id: String) -> Result<bool, AppError> {
    let rt = rt.inner().clone();
    blocking(move || {
        let g = rt.job.lock().map_err(|_| AppError::Runtime("internal state error".into()))?;
        match &*g {
            Some(j) if j.id == job_id => {
                j.cancel.cancel();
                Ok(true)
            }
            _ => Ok(false),
        }
    })
    .await
}

pub(crate) fn require_regular_distro(report: &WslReport, name: &str) -> Result<(), AppError> {
    match report.distros.iter().find(|d| d.name == name) {
        Some(d) if d.kind == DistroKind::Regular => Ok(()),
        Some(_) => Err(AppError::Runtime(format!("'{name}' belongs to another application and is never touched by VLINUX."))),
        None => Err(AppError::Runtime(format!("No distribution named '{name}' is registered with WSL."))),
    }
}

pub(crate) fn ensure_wsl_ready(report: &WslReport) -> Result<(), AppError> {
    match report.status {
        RuntimeStatus::Ready => Ok(()),
        _ => Err(AppError::Runtime("WSL is not ready. Check the setup status first.".into())),
    }
}

/// Where each shared folder would point, resolved now from the saved preferences.
#[tauri::command]
pub async fn storage_overview(config: State<'_, ConfigStore>) -> Result<StorageOverview, AppError> {
    let cfg = config
        .current
        .lock()
        .map_err(|_| AppError::Config("config lock poisoned".into()))?
        .storage
        .clone();
    blocking(move || Ok(overview_for(&cfg))).await
}

fn overview_for(cfg: &storage::StorageConfig) -> StorageOverview {
    let resolver: &dyn KnownFolderResolver = &SystemKnownFolders;
    let drives: &dyn DriveInspector = &SystemDrives;
    storage::overview(cfg, resolver, drives, storage::DEFAULT_MOUNT_ROOT)
}

/// Check one candidate Windows folder (UNC, device paths, ... are reported, not mapped).
#[tauri::command]
pub async fn storage_validate_path(path: String) -> Result<Verdict, AppError> {
    blocking(move || Ok(storage::windows_to_wsl(&path, storage::DEFAULT_MOUNT_ROOT, &SystemDrives))).await
}
