//! Tauri bindings for the Distribution Manager (M5).
//!
//! Rules enforced here (the logic itself lives in the Tauri-free `vlinux-runtime` crate):
//! * Nothing runs at startup and nothing polls. Every command is an explicit user action; the
//!   overview is built from the cached WSL report plus cached inspections.
//! * The frontend never supplies a command line, a path, or a package-manager argument vector. It
//!   names a distribution (validated against the current WSL state), a typed request, and values
//!   that the backend validates again.
//! * Lifecycle: one operation per distribution at a time; stop needs `confirmed`; no global
//!   shutdown, unregister, default change, import, export or mount exists anywhere in this code.
//! * Package operations are never executed here: `distro_pkg_plan` returns the exact command text
//!   for the user to run in the Terminal. Search is a read-only probe.
//! * `/etc/wsl.conf` edits are previewed, confirmed, backed up and reversible, and the backend
//!   recomputes the change from the current file instead of trusting frontend text.
//! * Diagnostics are explicit, bounded and cancellable; writing checks need explicit permission.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use vlinux_runtime::adapters::DistroRegistry;
use vlinux_runtime::diag::{self, CheckResult, DiagEnv, DiagSink, Mode, Options, Report};
use vlinux_runtime::distroconf::{
    self, ApplyOutcome, BackupStore, Change, ConfError, ConfFile, ConfView, HostConfFile, Plan,
};
use vlinux_runtime::exec::{CancelToken, CommandRunner, SystemRunner};
use vlinux_runtime::inspect::{self, InspectOptions, InspectOutcome, InspectionStore};
use vlinux_runtime::lifecycle::{self, LifecycleError, OpKind, OpOutcome, OpRegistry};
use vlinux_runtime::model::{DistroState, WslReport};
use vlinux_runtime::overview::{build_overview, Overview};
use vlinux_runtime::packages::{
    parse_search, valid_search_query, PackageHit, PackageOp, PackagePlan,
};
use vlinux_runtime::parse::valid_distro_name;
use vlinux_runtime::probe::{valid_win_probe_path, Prober};
use vlinux_runtime::shared::{DistroOps, MappingStore, WslDistroOps};
use vlinux_runtime::storage::{StorageConfig, SystemDrives, SystemKnownFolders};
use vlinux_runtime::terminal::{NativePty, TerminalManager};
use vlinux_runtime::unix_now;
use vlinux_runtime::winconf::{self, WindowsConfig};

use crate::config::{self, ConfigStore};
use crate::error::AppError;
use crate::files::linux_root;
use crate::runtime::{blocking, ensure_wsl_ready, require_regular_distro, RuntimeState};

pub const DISTRO_OP_EVENT: &str = "vlinux://distro-op";
pub const DIAG_EVENT: &str = "vlinux://diag-progress";

const DIAG_BUDGET: Duration = Duration::from_secs(120);
const DIAG_STEP: Duration = Duration::from_secs(8);
/// The first terminal answer may wait for the shell to start (profile scripts, systemd).
const DIAG_STARTUP: Duration = Duration::from_secs(20);

type Dm<'a> = State<'a, Arc<DistroMgmt>>;
type Cfg<'a> = State<'a, ConfigStore>;

#[derive(Serialize, Clone)]
struct OpEvent {
    distro: String,
    op: OpKind,
    /// `started` or `finished`.
    phase: &'static str,
    ok: bool,
    message: String,
}

#[derive(Serialize, Clone)]
struct DiagProgress {
    run_id: String,
    index: usize,
    result: CheckResult,
}

pub struct DistroMgmt {
    app: AppHandle,
    rt: Arc<RuntimeState>,
    runner: Arc<dyn CommandRunner>,
    registry: DistroRegistry,
    ops: Arc<OpRegistry>,
    inspections: InspectionStore,
    backups: BackupStore,
    /// The last failed start/stop/inspect per distribution; cleared by a success or a refresh.
    failures: Mutex<HashMap<String, String>>,
    /// Running inspections, so that a superseded one can be cancelled.
    cancels: Mutex<HashMap<String, CancelToken>>,
    diag: Mutex<Option<CancelToken>>,
    counter: AtomicU64,
    /// Serialises `/etc/wsl.conf` writes.
    conf_gate: Mutex<()>,
    dir: PathBuf,
}

impl DistroMgmt {
    pub fn new(app: AppHandle, rt: Arc<RuntimeState>, config_dir: &Path) -> Self {
        Self {
            app,
            rt,
            runner: Arc::new(SystemRunner),
            registry: DistroRegistry::with_builtin(),
            ops: OpRegistry::new(),
            inspections: InspectionStore::new(config_dir),
            backups: BackupStore::new(config_dir),
            failures: Mutex::new(HashMap::new()),
            cancels: Mutex::new(HashMap::new()),
            diag: Mutex::new(None),
            counter: AtomicU64::new(1),
            conf_gate: Mutex::new(()),
            dir: config_dir.to_path_buf(),
        }
    }

    fn failures(&self) -> HashMap<String, String> {
        self.failures.lock().map(|g| g.clone()).unwrap_or_default()
    }

    fn set_failure(&self, name: &str, message: Option<String>) {
        if let Ok(mut g) = self.failures.lock() {
            match message {
                Some(m) => {
                    g.insert(name.to_string(), m);
                }
                None => {
                    g.remove(name);
                }
            }
        }
    }

    fn emit_op(&self, distro: &str, op: OpKind, phase: &'static str, ok: bool, message: String) {
        let _ = self.app.emit(
            DISTRO_OP_EVENT,
            OpEvent {
                distro: distro.to_string(),
                op,
                phase,
                ok,
                message,
            },
        );
    }

    fn overview(&self, force: bool, preferred: Option<&str>) -> Overview {
        let report = self.rt.detect(force);
        build_overview(
            &report,
            preferred,
            &self.inspections.all(),
            &self.ops.snapshot(),
            &self.failures(),
            &self.registry,
            unix_now(),
        )
    }

    /// Start or stop one distribution. Validation happens before anything is registered or emitted.
    fn lifecycle(
        &self,
        name: &str,
        kind: OpKind,
        confirmed: bool,
    ) -> Result<LifecycleResult, AppError> {
        let report = self.rt.detect(false);
        match kind {
            OpKind::Start => {
                lifecycle::check_start(&report, name).map_err(lc)?;
            }
            OpKind::Stop => {
                lifecycle::check_stop(&report, name, confirmed).map_err(lc)?;
            }
            OpKind::Inspect => return Err(AppError::Runtime("Not a lifecycle operation.".into())),
        }
        let _token = self.ops.begin(name, kind).map_err(lc)?;
        self.emit_op(
            name,
            kind,
            "started",
            true,
            format!("{} '{name}'…", capitalised(kind.verb())),
        );
        let detect = || {
            self.rt.invalidate();
            self.rt.detect(true)
        };
        let res = match kind {
            OpKind::Start => lifecycle::start(&self.rt.wsl, &report, name, &detect),
            _ => lifecycle::stop(&self.rt.wsl, &report, name, confirmed, &detect),
        };
        match res {
            Ok(outcome) => {
                self.set_failure(name, None);
                self.emit_op(name, kind, "finished", true, outcome.message.clone());
                Ok(LifecycleResult {
                    outcome,
                    report: self.rt.detect(false),
                })
            }
            Err(e) => {
                if matches!(e, LifecycleError::Failed(_)) {
                    self.set_failure(name, Some(e.to_string()));
                }
                // WSL's state may have changed even though the operation failed.
                self.rt.invalidate();
                self.emit_op(name, kind, "finished", false, e.to_string());
                Err(lc(e))
            }
        }
    }

    fn inspect(&self, name: &str, allow_start: bool) -> Result<InspectOutcome, AppError> {
        let report = self.rt.detect(false);
        lifecycle::check_target(&report, name).map_err(lc)?;
        let _token = self.ops.begin(name, OpKind::Inspect).map_err(lc)?;
        let cancel = CancelToken::new();
        if let Ok(mut g) = self.cancels.lock() {
            g.insert(name.to_string(), cancel.clone());
        }
        self.emit_op(
            name,
            OpKind::Inspect,
            "started",
            true,
            format!("Inspecting '{name}'…"),
        );
        let res = inspect::inspect(
            &self.runner,
            &self.rt.programs.wsl,
            &report,
            name,
            InspectOptions { allow_start },
            &cancel,
            &self.registry,
            unix_now(),
        );
        if let Ok(mut g) = self.cancels.lock() {
            g.remove(name);
        }
        match res {
            Ok(InspectOutcome::Done { inspection }) => {
                let mut note = String::new();
                if let Err(e) = self.inspections.put(inspection.clone()) {
                    note = format!(" (It could not be saved for next time: {e}.)");
                }
                if inspection.started_by_inspection {
                    self.rt.invalidate();
                }
                self.set_failure(name, None);
                self.emit_op(
                    name,
                    OpKind::Inspect,
                    "finished",
                    true,
                    format!("Inspected '{name}'.{note}"),
                );
                Ok(InspectOutcome::Done { inspection })
            }
            Ok(other) => {
                self.emit_op(
                    name,
                    OpKind::Inspect,
                    "finished",
                    true,
                    "Not inspected: the distribution is stopped.".into(),
                );
                Ok(other)
            }
            Err(e) => {
                if matches!(e, LifecycleError::Failed(_)) {
                    self.set_failure(name, Some(e.to_string()));
                }
                self.rt.invalidate();
                self.emit_op(name, OpKind::Inspect, "finished", false, e.to_string());
                Err(lc(e))
            }
        }
    }

    /// Package support for an inspected distribution; never assumed.
    fn package_support(
        &self,
        report: &WslReport,
        name: &str,
    ) -> Result<(vlinux_runtime::packages::PackageSupport, Option<String>), AppError> {
        let distro = lifecycle::check_target(report, name).map_err(lc)?;
        let adapter = self
            .registry
            .by_id(&distro.adapter.id)
            .unwrap_or_else(|| self.registry.generic());
        let insp = self
            .inspections
            .get(name)
            .ok_or_else(|| AppError::Runtime(format!("'{name}' has not been inspected yet. Inspect it first so VLINUX can see which package manager it really has.")))?;
        let (support, availability, _notes) =
            inspect::resolve_packages(adapter.packages(), &insp.managers);
        match availability {
            vlinux_runtime::packages::Availability::Present { version } => Ok((support, version)),
            vlinux_runtime::packages::Availability::Absent => Err(AppError::Runtime(
                "No package manager VLINUX supports was found in this distribution.".into(),
            )),
            vlinux_runtime::packages::Availability::NotChecked => Err(AppError::Runtime(
                "The package manager was not checked yet. Inspect the distribution first.".into(),
            )),
        }
    }

    fn conf_file(&self, name: &str) -> Result<(HostConfFile, u8), ConfError> {
        let report = self.rt.detect(false);
        let wsl_version = distroconf::check_accessible(&report, name)?;
        let root = linux_root(name).map_err(|e| ConfError::Invalid(e.message))?;
        Ok((
            HostConfFile::new(root.join("etc").join("wsl.conf")),
            wsl_version,
        ))
    }
}

fn lc(e: LifecycleError) -> AppError {
    AppError::Runtime(e.to_string())
}

fn conf_err(e: ConfError) -> AppError {
    AppError::Runtime(e.to_string())
}

fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

// ---------------------------------------------------------------------------------------------
// Overview and the preferred distribution
// ---------------------------------------------------------------------------------------------

fn preferred_of(cfg: &Cfg<'_>) -> Result<Option<String>, AppError> {
    Ok(cfg
        .current
        .lock()
        .map_err(|_| AppError::Config("config lock poisoned".into()))?
        .runtime
        .selected_distro
        .clone())
}

/// Cards for every distribution plus the preferred-distribution status. `force` is the explicit
/// "Refresh": it re-lists WSL's distributions and forgets earlier failures. Never starts anything.
#[tauri::command]
pub async fn distro_overview(
    dm: Dm<'_>,
    cfg: Cfg<'_>,
    force: Option<bool>,
) -> Result<Overview, AppError> {
    let dm = dm.inner().clone();
    let preferred = preferred_of(&cfg)?;
    let force = force.unwrap_or(false);
    blocking(move || {
        if force {
            if let Ok(mut g) = dm.failures.lock() {
                g.clear();
            }
        }
        Ok(dm.overview(force, preferred.as_deref()))
    })
    .await
}

/// Choose (or clear, with `None`) the distribution VLINUX uses. Stored in VLINUX's own config only:
/// the Windows default distribution is never changed, and open terminals are not affected.
#[tauri::command]
pub async fn distro_set_preferred(
    dm: Dm<'_>,
    cfg: Cfg<'_>,
    name: Option<String>,
) -> Result<Overview, AppError> {
    let dm = dm.inner().clone();
    if let Some(n) = &name {
        if !valid_distro_name(n) {
            return Err(AppError::Runtime(
                "That is not a valid distribution name.".into(),
            ));
        }
    }
    let check_name = name.clone();
    let rt = dm.rt.clone();
    blocking(move || {
        if let Some(n) = &check_name {
            let report = rt.detect(false);
            ensure_wsl_ready(&report)?;
            require_regular_distro(&report, n)?;
        }
        Ok(())
    })
    .await?;
    {
        let mut cur = cfg
            .current
            .lock()
            .map_err(|_| AppError::Config("config lock poisoned".into()))?;
        let mut next = cur.clone();
        next.runtime.selected_distro = name.clone();
        let next = next.sanitized();
        config::save(&cfg.path, &next)?;
        *cur = next;
    }
    blocking(move || Ok(dm.overview(false, name.as_deref()))).await
}

// ---------------------------------------------------------------------------------------------
// Lifecycle and inspection
// ---------------------------------------------------------------------------------------------

#[derive(Serialize)]
pub struct LifecycleResult {
    pub outcome: OpOutcome,
    pub report: WslReport,
}

/// Start one distribution (explicit user action). Progress: `vlinux://distro-op` events; the card
/// shows "Starting" while it runs. A second start of the same distribution is refused.
#[tauri::command]
pub async fn distro_start(dm: Dm<'_>, name: String) -> Result<LifecycleResult, AppError> {
    let dm = dm.inner().clone();
    blocking(move || dm.lifecycle(&name, OpKind::Start, true)).await
}

/// Stop one distribution. Needs `confirmed = true` (the UI shows an interruption warning first).
/// This stops only the named distribution: there is no global shutdown.
#[tauri::command]
pub async fn distro_stop(
    dm: Dm<'_>,
    name: String,
    confirmed: bool,
) -> Result<LifecycleResult, AppError> {
    let dm = dm.inner().clone();
    blocking(move || dm.lifecycle(&name, OpKind::Stop, confirmed)).await
}

/// Read-only inspection. A stopped distribution is started only with `allow_start`.
#[tauri::command]
pub async fn distro_inspect(
    dm: Dm<'_>,
    name: String,
    allow_start: Option<bool>,
) -> Result<InspectOutcome, AppError> {
    let dm = dm.inner().clone();
    blocking(move || dm.inspect(&name, allow_start.unwrap_or(false))).await
}

/// Cancel a running inspection of this distribution. Returns whether one was running.
#[tauri::command]
pub async fn distro_inspect_cancel(dm: Dm<'_>, name: String) -> Result<bool, AppError> {
    let dm = dm.inner().clone();
    blocking(move || {
        let g = dm
            .cancels
            .lock()
            .map_err(|_| AppError::Runtime("internal state error".into()))?;
        match g.get(&name) {
            Some(t) => {
                t.cancel();
                Ok(true)
            }
            None => Ok(false),
        }
    })
    .await
}

// ---------------------------------------------------------------------------------------------
// Packages: plans (never executed) and read-only search
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageRequest {
    Install,
    Update,
}

/// The exact commands for an install or update, for the user to review and run in the Terminal.
/// Nothing is executed and no privilege escalation is attempted.
#[tauri::command]
pub async fn distro_pkg_plan(
    dm: Dm<'_>,
    name: String,
    op: PackageRequest,
    packages: Vec<String>,
) -> Result<PackagePlan, AppError> {
    let dm = dm.inner().clone();
    blocking(move || {
        let report = dm.rt.detect(false);
        let (support, _version) = dm.package_support(&report, &name)?;
        let op = match op {
            PackageRequest::Install => PackageOp::Install,
            PackageRequest::Update => PackageOp::Update,
        };
        support
            .plan(&name, op, &packages)
            .map_err(AppError::Runtime)
    })
    .await
}

/// Search the distribution's own package index (read-only). The distribution must be running.
#[tauri::command]
pub async fn distro_pkg_search(
    dm: Dm<'_>,
    name: String,
    query: String,
) -> Result<Vec<PackageHit>, AppError> {
    let dm = dm.inner().clone();
    blocking(move || {
        let report = dm.rt.detect(false);
        let distro = lifecycle::check_target(&report, &name).map_err(lc)?;
        if distro.state != DistroState::Running {
            return Err(AppError::Runtime(format!(
                "'{name}' is stopped. Start it first: searching never starts a distribution."
            )));
        }
        if !valid_search_query(&query) {
            return Err(AppError::Runtime(
                "Use 1–64 letters, digits, and . _ + - in the search.".into(),
            ));
        }
        let (support, _) = dm.package_support(&report, &name)?;
        let argv = support.search_argv(&query).ok_or_else(|| {
            AppError::Runtime("Searching is not supported for this package manager.".into())
        })?;
        let cancel = CancelToken::new();
        let prober = Prober::new(&dm.runner, &dm.rt.programs.wsl, &name, &cancel)
            .map_err(AppError::Runtime)?;
        let out = prober
            .search(&argv)
            .map_err(|e| AppError::Runtime(format!("The search could not run: {e}.")))?;
        // Package managers exit 1 for "no match"; anything else is a real failure.
        if !out.success() && out.status != Some(1) {
            return Err(AppError::Runtime(
                "The package manager reported an error. Its index may be out of date or locked."
                    .into(),
            ));
        }
        Ok(parse_search(support.manager, &Prober::text(&out)))
    })
    .await
}

// ---------------------------------------------------------------------------------------------
// Configuration: /etc/wsl.conf (per distribution) and .wslconfig (Windows, read-only)
// ---------------------------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WslConfRead {
    View {
        view: ConfView,
        wsl_version: u8,
    },
    /// Reaching the file starts a stopped distribution, so VLINUX waits for an explicit Start.
    NeedsStart {
        distro: String,
    },
}

#[tauri::command]
pub async fn wslconf_read(dm: Dm<'_>, name: String) -> Result<WslConfRead, AppError> {
    let dm = dm.inner().clone();
    blocking(move || {
        let (file, wsl_version) = match dm.conf_file(&name) {
            Ok(v) => v,
            Err(ConfError::NotRunning(d)) => return Ok(WslConfRead::NeedsStart { distro: d }),
            Err(e) => return Err(conf_err(e)),
        };
        let text = file.read().map_err(conf_err)?;
        Ok(WslConfRead::View {
            view: distroconf::view(&name, text.as_deref()),
            wsl_version,
        })
    })
    .await
}

/// A reviewable diff for the requested settings. Writes nothing.
#[tauri::command]
pub async fn wslconf_preview(
    dm: Dm<'_>,
    name: String,
    changes: Vec<Change>,
) -> Result<Plan, AppError> {
    let dm = dm.inner().clone();
    blocking(move || {
        let (file, wsl_version) = dm.conf_file(&name).map_err(conf_err)?;
        let text = file.read().map_err(conf_err)?;
        distroconf::plan(&name, text.as_deref(), &changes, wsl_version).map_err(conf_err)
    })
    .await
}

/// Apply the previewed change. The backend recomputes the new file from the *current* file and the
/// requested settings (it never accepts file text from the frontend) and refuses if the file is no
/// longer the one that was previewed. A backup is taken first. Needs `confirmed = true`.
#[tauri::command]
pub async fn wslconf_apply(
    dm: Dm<'_>,
    name: String,
    changes: Vec<Change>,
    based_on: String,
    confirmed: bool,
) -> Result<ApplyOutcome, AppError> {
    let dm = dm.inner().clone();
    blocking(move || {
        if !confirmed {
            return Err(AppError::Runtime(
                "The change was not confirmed. Nothing was written.".into(),
            ));
        }
        let _gate = dm.conf_gate.lock().unwrap_or_else(|p| p.into_inner());
        let (file, wsl_version) = dm.conf_file(&name).map_err(conf_err)?;
        let text = file.read().map_err(conf_err)?;
        let plan =
            distroconf::plan(&name, text.as_deref(), &changes, wsl_version).map_err(conf_err)?;
        if plan.based_on != based_on {
            return Err(conf_err(ConfError::Changed));
        }
        distroconf::apply(
            &file,
            &dm.backups,
            &name,
            &plan.based_on,
            &plan.new_text,
            unix_now(),
        )
        .map_err(conf_err)
    })
    .await
}

#[derive(Serialize)]
pub struct BackupInfo {
    pub id: String,
    pub taken_at: u64,
    /// The file existed before the change (restoring brings it back; otherwise restoring removes it).
    pub existed: bool,
    pub applied: bool,
    pub restored_at: Option<u64>,
    pub bytes: usize,
}

#[tauri::command]
pub async fn wslconf_backups(dm: Dm<'_>, name: String) -> Result<Vec<BackupInfo>, AppError> {
    let dm = dm.inner().clone();
    blocking(move || {
        if !valid_distro_name(&name) {
            return Err(AppError::Runtime(
                "That is not a valid distribution name.".into(),
            ));
        }
        let mut v: Vec<BackupInfo> = dm
            .backups
            .list(&name)
            .into_iter()
            .map(|b| BackupInfo {
                id: b.id,
                taken_at: b.taken_at,
                existed: b.existed,
                applied: b.written_fp.is_some(),
                restored_at: b.restored_at,
                bytes: b.content.len(),
            })
            .collect();
        v.sort_by(|a, b| b.taken_at.cmp(&a.taken_at));
        Ok(v)
    })
    .await
}

/// Put a backup back. A safety backup of the current file is taken first, so this is undoable.
/// Refuses when the file was changed by someone else since, unless `force`. Needs `confirmed`.
#[tauri::command]
pub async fn wslconf_restore(
    dm: Dm<'_>,
    name: String,
    backup_id: String,
    force: Option<bool>,
    confirmed: bool,
) -> Result<ApplyOutcome, AppError> {
    let dm = dm.inner().clone();
    blocking(move || {
        if !confirmed {
            return Err(AppError::Runtime(
                "The restore was not confirmed. Nothing was changed.".into(),
            ));
        }
        let _gate = dm.conf_gate.lock().unwrap_or_else(|p| p.into_inner());
        let (file, _) = dm.conf_file(&name).map_err(conf_err)?;
        distroconf::restore(
            &file,
            &dm.backups,
            &name,
            &backup_id,
            force.unwrap_or(false),
            unix_now(),
        )
        .map_err(conf_err)
    })
    .await
}

/// The Windows-side `.wslconfig` (shared by every distribution). Read-only.
#[tauri::command]
pub async fn wslconfig_windows() -> Result<WindowsConfig, AppError> {
    blocking(move || {
        let path = winconf::default_path();
        let cpus = std::thread::available_parallelism().ok().map(|n| n.get());
        Ok(winconf::read(path.as_deref(), cpus))
    })
    .await
}

// ---------------------------------------------------------------------------------------------
// Diagnostics
// ---------------------------------------------------------------------------------------------

struct DiagGuard<'a>(&'a DistroMgmt);

impl Drop for DiagGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut g) = self.0.diag.lock() {
            *g = None;
        }
    }
}

fn diag_token(seq: u64) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    // 12 lower-case hex digits: unique enough for a folder name that lives for seconds.
    format!(
        "vlx-{:012x}",
        (nanos ^ seq.wrapping_mul(0x9E37_79B9_7F4A_7C15)) & 0xFFFF_FFFF_FFFF
    )
}

impl DistroMgmt {
    fn run_diag(&self, options: &Options, storage: &StorageConfig, cancel: &CancelToken) -> Report {
        let seq = self.counter.fetch_add(1, Ordering::SeqCst);
        let run_id = format!("diag-{seq}");
        let extra_scrub: Vec<String> = options
            .distro
            .as_deref()
            .and_then(|n| self.inspections.get(n))
            .and_then(|i| i.default_user)
            .into_iter()
            .collect();
        let app = self.app.clone();
        let mut index = 0usize;
        run_diag_core(
            &self.rt,
            &self.runner,
            &self.dir,
            extra_scrub,
            seq,
            options,
            storage,
            cancel,
            &mut |c: &CheckResult| {
                index += 1;
                let _ = app.emit(
                    DIAG_EVENT,
                    DiagProgress {
                        run_id: run_id.clone(),
                        index,
                        result: c.clone(),
                    },
                );
            },
        )
    }
}

/// The diagnostics run itself, free of any window: the app calls it with an event-emitting
/// progress callback, and the WSL lab (a disposable CI runner with a real Arch) calls it directly.
#[allow(clippy::too_many_arguments)]
fn run_diag_core(
    rt: &Arc<RuntimeState>,
    runner: &Arc<dyn CommandRunner>,
    dir: &Path,
    extra_scrub: Vec<String>,
    seq: u64,
    options: &Options,
    storage: &StorageConfig,
    cancel: &CancelToken,
    progress: &mut dyn FnMut(&CheckResult),
) -> Report {
    {
        let report = rt.detect(true);

        let running = options
            .distro
            .as_deref()
            .and_then(|n| report.distros.iter().find(|d| d.name == n))
            .map(|d| d.state == DistroState::Running)
            .unwrap_or(false);
        // Looking for the share of a stopped distribution would start it, so the path is only looked
        // up for a running one. With permission to start, the standard path is used (the share is
        // only listed after the Runtime checks have started and verified the distribution).
        let root: Option<PathBuf> = match options.distro.as_deref() {
            Some(n) if valid_distro_name(n) => {
                if running {
                    linux_root(n).ok()
                } else if options.allow_start && cfg!(windows) {
                    Some(PathBuf::from(format!(r"\\wsl.localhost\{n}")))
                } else {
                    None
                }
            }
            _ => None,
        };
        let ops = options.distro.as_deref().and_then(|n| {
            WslDistroOps::new(runner.clone(), rt.programs.wsl.clone(), n).ok()
        });
        let sink = Arc::new(DiagSink::new());
        let mgr = TerminalManager::new(Arc::new(NativePty), sink.clone());
        let detect = || {
            rt.invalidate();
            rt.detect(true)
        };
        let exists = |p: &str| Path::new(p).is_dir();
        let store = MappingStore::new(&dir);
        let resolver = SystemKnownFolders;
        let drives = SystemDrives;
        let sysroot = if cfg!(windows) {
            std::env::var("SystemRoot").ok()
        } else {
            None
        };
        let explorer = sysroot
            .as_ref()
            .map(|r| PathBuf::from(r).join("explorer.exe"));
        let probe = sysroot
            .as_ref()
            .map(|r| r.replace('\\', "/"))
            .filter(|p| valid_win_probe_path(p));
        let mut scrub: Vec<String> = ["USERNAME", "USER", "COMPUTERNAME", "HOSTNAME"]
            .iter()
            .filter_map(|k| std::env::var(k).ok())
            .collect();
        scrub.extend(extra_scrub);
        let env = DiagEnv {
            mode: Mode::Real,
            report: &report,
            runner,
            programs: &rt.programs,
            terminal: Some((&mgr, &*sink)),
            linux_root: root.as_deref(),
            ops: ops.as_ref().map(|o| o as &dyn DistroOps),
            resolver: &resolver,
            drives: &drives,
            mapping_store: &store,
            storage,
            detect: &detect,
            windows_dir_exists: &exists,
            explorer,
            windows_probe_dir: probe,
            now: unix_now(),
            token: diag_token(seq),
            budget: DIAG_BUDGET,
            step_timeout: DIAG_STEP,
            startup_timeout: DIAG_STARTUP,
            scrub,
        };
        let result = diag::run(&env, options, cancel, progress);
        mgr.close_all();
        result
    }
}

/// Run the compatibility diagnostics for one distribution. One run at a time. Progress arrives as
/// `vlinux://diag-progress`. Checks that write need `allow_write`; a stopped distribution is
/// started only with `allow_start`.
#[tauri::command]
pub async fn diag_run(dm: Dm<'_>, cfg: Cfg<'_>, options: Options) -> Result<Report, AppError> {
    let dm = dm.inner().clone();
    let storage = cfg
        .current
        .lock()
        .map_err(|_| AppError::Config("config lock poisoned".into()))?
        .storage
        .clone();
    blocking(move || {
        let cancel = CancelToken::new();
        {
            let mut g = dm
                .diag
                .lock()
                .map_err(|_| AppError::Runtime("internal state error".into()))?;
            if g.is_some() {
                return Err(AppError::Runtime(
                    "A diagnostics run is already in progress.".into(),
                ));
            }
            *g = Some(cancel.clone());
        }
        let _guard = DiagGuard(&dm);
        Ok(dm.run_diag(&options, &storage, &cancel))
    })
    .await
}

/// Ask the running diagnostics to stop. Clean-up still runs. Returns whether a run was active.
#[tauri::command]
pub async fn diag_cancel(dm: Dm<'_>) -> Result<bool, AppError> {
    let dm = dm.inner().clone();
    blocking(move || {
        let g = dm
            .diag
            .lock()
            .map_err(|_| AppError::Runtime("internal state error".into()))?;
        match &*g {
            Some(t) => {
                t.cancel();
                Ok(true)
            }
            None => Ok(false),
        }
    })
    .await
}

/// The WSL lab: runs the REAL diagnostics, through the same code path as the app, against a real
/// distribution. It does nothing unless `VLINUX_LAB_DISTRO` is set, which only the lab workflow
/// (a disposable GitHub-hosted VM with a throw-away Arch) does. Never run it against a distribution
/// you care about: the diagnostics create and remove one folder under /tmp and type into a terminal.
#[cfg(test)]
mod lab {
    use super::*;

    #[test]
    fn real_diagnostics_in_the_wsl_lab() {
        let Ok(distro) = std::env::var("VLINUX_LAB_DISTRO") else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("vlinux-lab-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let rt = Arc::new(RuntimeState::new(&dir));
        let runner: Arc<dyn CommandRunner> = Arc::new(SystemRunner);
        let options = Options {
            distro: Some(distro),
            allow_start: true,
            allow_write: true,
            categories: None,
        };
        let report = run_diag_core(
            &rt,
            &runner,
            &dir,
            Vec::new(),
            1,
            &options,
            &StorageConfig::default(),
            &CancelToken::new(),
            &mut |c: &CheckResult| println!("LAB-CHECK {} {:?} {} ms", c.id, c.outcome, c.ms),
        );
        for c in &report.results {
            println!(
                "LAB-RESULT {} | {:?} | {} ms | {} | {}",
                c.id,
                c.outcome,
                c.ms,
                c.message.replace('\n', " "),
                c.guidance.clone().unwrap_or_default().replace('\n', " ")
            );
        }
        println!("LAB-SUMMARY {:?} leftover={:?}", report.summary, report.diag_dir);
        if let Ok(out) = std::env::var("VLINUX_LAB_OUT") {
            let _ = std::fs::write(out, serde_json::to_string_pretty(&report).unwrap_or_default());
        }
        // The lab reports instead of asserting: its purpose is evidence, not a verdict.
    }
}
