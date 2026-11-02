//! Tauri bindings for the file manager and the shared-folder mappings.
//!
//! * The frontend never sends a host path. It sends a typed `Location`: a Linux path for the
//!   selected distribution, or a shared Windows folder id plus relative components. The runtime
//!   crate validates and resolves it (`fs::Spaces`); anything outside the allowed places is refused.
//! * Every command is `async` and does its filesystem work on a worker thread. Long operations
//!   (copy, move, delete) run as jobs that report progress through `vlinux://fs-job` events.
//! * Nothing happens at startup. A Linux process is started only when the user opens a distribution
//!   in Files or checks/applies a mapping.
//! * VLINUX's own configuration and app data are never reachable from here.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use vlinux_runtime::exec::{CancelToken, CommandRunner, SystemRunner};
use vlinux_runtime::fs::filetype::open_policy;
use vlinux_runtime::fs::{
    ops, Conflict, FsError, FsErrorKind, FsResult, ListOptions, ListStore, ListingHead, Location,
    OpReport, Page, Progress, Runner, Spaces, TransferMode, WindowsRoot,
};
use vlinux_runtime::parse::valid_distro_name;
use vlinux_runtime::shared::engine::{self, same_path, Ctx};
use vlinux_runtime::shared::{
    parse_wsl_conf, DistroOps, MappingStatus, MappingStore, Resolution, WslConf, WslDistroOps,
};
use vlinux_runtime::storage::{
    windows_to_wsl, KnownFolder, KnownFolderResolver, StorageConfig, SystemDrives,
    SystemKnownFolders,
};
use vlinux_runtime::unix_now;

use crate::config::ConfigStore;
use crate::runtime::{ensure_wsl_ready, require_regular_distro, RuntimeState};

pub const JOB_EVENT: &str = "vlinux://fs-job";
const MAX_JOBS: usize = 6;
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

fn unavailable(msg: impl Into<String>) -> FsError {
    FsError::new(FsErrorKind::Unavailable, msg)
}

/// What is known about one distribution, gathered once per explicit refresh.
struct DistroInfo {
    home: String,
    conf: WslConf,
    root: PathBuf,
    /// Recorded mappings whose link was *verified live*: (linux path, folder id, target).
    links: Vec<(String, String, String)>,
}

pub struct FilesState {
    app: AppHandle,
    rt: Arc<RuntimeState>,
    runner: Arc<dyn CommandRunner>,
    lists: ListStore,
    jobs: Mutex<HashMap<String, CancelToken>>,
    counter: AtomicU64,
    info: Mutex<HashMap<String, Arc<DistroInfo>>>,
    /// Serialises mapping checks, applies and undos.
    gate: Mutex<()>,
    store: MappingStore,
}

type Fs<'a> = State<'a, Arc<FilesState>>;
type Cfg<'a> = State<'a, ConfigStore>;

async fn run<T, F>(f: F) -> Result<T, FsError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, FsError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| FsError::new(FsErrorKind::Io, format!("background task failed: {e}")))?
}

fn storage_of(cfg: &Cfg<'_>) -> Result<StorageConfig, FsError> {
    cfg.current
        .lock()
        .map(|c| c.storage.clone())
        .map_err(|_| FsError::new(FsErrorKind::Io, "configuration lock poisoned"))
}

/// Host path of a distribution's file share.
pub(crate) fn linux_root(distro: &str) -> FsResult<PathBuf> {
    if !valid_distro_name(distro) {
        return Err(FsError::new(
            FsErrorKind::InvalidPath,
            format!("'{distro}' is not a valid distribution name."),
        ));
    }
    #[cfg(windows)]
    {
        let modern = PathBuf::from(format!(r"\\wsl.localhost\{distro}"));
        if modern.exists() {
            return Ok(modern);
        }
        let legacy = PathBuf::from(format!(r"\\wsl$\{distro}"));
        if legacy.exists() {
            return Ok(legacy);
        }
        Err(unavailable(format!(
            "Can't reach the files of '{distro}'. Make sure it is installed and can start (open Linux Setup to check)."
        )))
    }
    #[cfg(not(windows))]
    {
        Err(unavailable(
            "Linux files are reached through WSL, which is only available on Windows.",
        ))
    }
}

fn windows_roots(cfg: &StorageConfig, mount_root: &str, automount: bool) -> Vec<WindowsRoot> {
    let resolver = SystemKnownFolders;
    let drives = SystemDrives;
    KnownFolder::ALL
        .iter()
        .filter_map(|&f| {
            let c = cfg.folder(f);
            let custom = if c.mode == vlinux_runtime::storage::MappingMode::Shared {
                c.windows_path.clone()
            } else {
                None
            };
            let path = custom.or_else(|| resolver.resolve(f))?;
            let linux_mount = if automount {
                windows_to_wsl(&path, mount_root, &drives)
                    .wsl_path()
                    .map(|s| s.to_string())
            } else {
                None
            };
            Some(WindowsRoot {
                id: f.id().to_string(),
                label: f.label().to_string(),
                dir: PathBuf::from(path),
                linux_mount,
            })
        })
        .collect()
}

fn is_unc(p: &Path) -> bool {
    let s = p.to_string_lossy();
    s.starts_with("\\\\") && !s.starts_with("\\\\?\\")
}

fn uses_linux(locs: &[&Location]) -> bool {
    locs.iter().any(|l| matches!(l, Location::Linux { .. }))
}

impl FilesState {
    pub fn new(app: AppHandle, rt: Arc<RuntimeState>, config_dir: &Path) -> Self {
        Self {
            app,
            rt,
            runner: Arc::new(SystemRunner),
            lists: ListStore::new(),
            jobs: Mutex::new(HashMap::new()),
            counter: AtomicU64::new(1),
            info: Mutex::new(HashMap::new()),
            gate: Mutex::new(()),
            store: MappingStore::new(config_dir),
        }
    }

    fn distro_ops(&self, distro: &str) -> FsResult<WslDistroOps> {
        WslDistroOps::new(self.runner.clone(), self.rt.programs.wsl.clone(), distro)
            .map_err(|e| unavailable(e.to_string()))
    }

    fn require_distro(&self, name: &str) -> FsResult<()> {
        let report = self.rt.detect(false);
        ensure_wsl_ready(&report).map_err(|e| unavailable(e.to_string()))?;
        require_regular_distro(&report, name).map_err(|e| unavailable(e.to_string()))
    }

    fn cached_info(&self, distro: &str) -> Option<Arc<DistroInfo>> {
        self.info.lock().ok()?.get(distro).cloned()
    }

    fn forget(&self, distro: &str) {
        if let Ok(mut g) = self.info.lock() {
            g.remove(distro);
        }
    }

    /// Home folder, wsl.conf and the verified mapped links of `distro`. Cached until refreshed;
    /// a refresh is the only thing that runs commands inside the distribution.
    fn info(&self, distro: &str, refresh: bool) -> FsResult<Arc<DistroInfo>> {
        if !refresh {
            if let Some(i) = self.cached_info(distro) {
                return Ok(i);
            }
        }
        self.require_distro(distro)?;
        let root = linux_root(distro)?;
        let ops = self.distro_ops(distro)?;
        let home = ops.home_dir().map_err(|e| {
            unavailable(format!("Couldn't find the home folder of '{distro}': {e}"))
        })?;
        let conf = std::fs::read_to_string(root.join("etc").join("wsl.conf"))
            .map(|t| parse_wsl_conf(&t))
            .unwrap_or_default();
        let mut links = Vec::new();
        for r in self.store.for_distro(distro) {
            if let Ok(Some(t)) = ops.read_link(&r.linux_path) {
                if same_path(&t, &r.target) {
                    links.push((r.linux_path.clone(), r.folder.clone(), r.target.clone()));
                }
            }
        }
        let info = Arc::new(DistroInfo {
            home,
            conf,
            root,
            links,
        });
        if let Ok(mut g) = self.info.lock() {
            g.insert(distro.to_string(), info.clone());
        }
        Ok(info)
    }

    fn spaces_from(&self, info: Option<&DistroInfo>, cfg: &StorageConfig) -> Spaces {
        match info {
            Some(i) => {
                let windows = windows_roots(cfg, &i.conf.mount_root, i.conf.automount_enabled);
                // A recorded link is trusted for navigation only while it was verified and the
                // Windows folder it points to is still the configured one.
                let mapped_links = i
                    .links
                    .iter()
                    .filter(|(_, id, target)| {
                        windows.iter().any(|r| {
                            &r.id == id
                                && r.linux_mount
                                    .as_deref()
                                    .is_some_and(|m| same_path(m, target))
                        })
                    })
                    .map(|(l, id, _)| (l.clone(), id.clone()))
                    .collect();
                Spaces {
                    linux_root: Some(i.root.clone()),
                    linux_home: Some(i.home.clone()),
                    mount_root: i.conf.mount_root.clone(),
                    windows,
                    mapped_links,
                }
            }
            None => Spaces {
                linux_root: None,
                linux_home: None,
                mount_root: "/mnt".into(),
                windows: windows_roots(cfg, "/mnt", false),
                mapped_links: Vec::new(),
            },
        }
    }

    /// The spaces for one request. A Linux problem only matters if the request needs Linux.
    fn spaces(
        &self,
        distro: Option<&str>,
        cfg: &StorageConfig,
        locs: &[&Location],
    ) -> FsResult<Spaces> {
        let needs_linux = uses_linux(locs);
        let info = match distro {
            Some(d) => match self.info(d, false) {
                Ok(i) => Some(i),
                Err(e) if needs_linux => return Err(e),
                Err(_) => None,
            },
            None => None,
        };
        Ok(self.spaces_from(info.as_deref(), cfg))
    }

    fn start_job<F>(self: &Arc<Self>, kind: &'static str, work: F) -> FsResult<JobStarted>
    where
        F: FnOnce(&mut Runner<'_>) -> FsResult<()> + Send + 'static,
    {
        let job_id = format!("fs-{}", self.counter.fetch_add(1, Ordering::SeqCst));
        let cancel = CancelToken::new();
        {
            let mut jobs = self
                .jobs
                .lock()
                .map_err(|_| FsError::new(FsErrorKind::Io, "internal state error"))?;
            if jobs.len() >= MAX_JOBS {
                return Err(FsError::new(
                    FsErrorKind::Refused,
                    "Too many file operations are running. Wait for one to finish.",
                ));
            }
            jobs.insert(job_id.clone(), cancel.clone());
        }
        let me = self.clone();
        let id = job_id.clone();
        let spawned = std::thread::Builder::new()
            .name("vlinux-fs-job".into())
            .spawn(move || {
                let app = me.app.clone();
                let emit = |state: &'static str,
                            progress: Option<Progress>,
                            report: Option<OpReport>,
                            error: Option<FsError>| {
                    let _ = app.emit(
                        JOB_EVENT,
                        JobEvent {
                            job_id: id.clone(),
                            kind,
                            state,
                            progress,
                            report,
                            error,
                        },
                    );
                };
                emit("running", None, None, None);
                let mut last = Instant::now();
                let mut on_progress = |p: &Progress| {
                    if last.elapsed() >= PROGRESS_EVERY {
                        last = Instant::now();
                        emit("running", Some(p.clone()), None, None);
                    }
                };
                let mut runner = Runner::new(&cancel, &mut on_progress);
                let result = work(&mut runner);
                let report = runner.finish();
                match result {
                    Err(e) if e.kind == FsErrorKind::Cancelled => {
                        emit("cancelled", None, Some(report), None)
                    }
                    Err(e) => emit("failed", None, Some(report), Some(e)),
                    Ok(()) if report.cancelled => emit("cancelled", None, Some(report), None),
                    Ok(()) => emit("done", None, Some(report), None),
                }
                if let Ok(mut jobs) = me.jobs.lock() {
                    jobs.remove(&id);
                }
            });
        if let Err(e) = spawned {
            if let Ok(mut jobs) = self.jobs.lock() {
                jobs.remove(&job_id);
            }
            return Err(FsError::new(
                FsErrorKind::Io,
                format!("couldn't start the operation: {e}"),
            ));
        }
        Ok(JobStarted { job_id })
    }
}

#[derive(Debug, Serialize)]
pub struct JobStarted {
    pub job_id: String,
}

#[derive(Debug, Clone, Serialize)]
struct JobEvent {
    job_id: String,
    kind: &'static str,
    state: &'static str,
    progress: Option<Progress>,
    report: Option<OpReport>,
    error: Option<FsError>,
}

// ---------------------------------------------------------------------------------------------
// Places
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct Place {
    id: String,
    label: String,
    location: Location,
    detail: Option<String>,
    /// Verified live (not merely preferred): this Linux folder is the Windows folder.
    linked: bool,
    unc: bool,
    warning: Option<String>,
    exists: bool,
}

#[derive(Debug, Serialize)]
pub struct Places {
    distro: Option<String>,
    linux_available: bool,
    linux_note: Option<String>,
    home: Option<Place>,
    roots: Vec<Place>,
    /// Home, Desktop, Documents, ... in the Linux home (or the Windows folder when there is no Linux).
    folders: Vec<Place>,
    shared: Vec<Place>,
}

const SIDEBAR_ORDER: [KnownFolder; 6] = [
    KnownFolder::Desktop,
    KnownFolder::Documents,
    KnownFolder::Downloads,
    KnownFolder::Pictures,
    KnownFolder::Music,
    KnownFolder::Videos,
];

/// The places for the sidebar. `refresh` re-reads the distribution (home folder, mappings).
#[tauri::command]
pub async fn fs_places(
    fs: Fs<'_>,
    cfg: Cfg<'_>,
    distro: Option<String>,
    refresh: Option<bool>,
) -> Result<Places, FsError> {
    let fs = fs.inner().clone();
    let cfg = storage_of(&cfg)?;
    run(move || {
        let (info, note) = match distro.as_deref() {
            Some(d) => match fs.info(d, refresh.unwrap_or(false)) {
                Ok(i) => (Some(i), None),
                Err(e) => (None, Some(e.message)),
            },
            None => (None, Some("No Linux distribution is selected. Choose one in Linux Setup to browse its files.".to_string())),
        };
        let spaces = fs.spaces_from(info.as_deref(), &cfg);
        let linked_link = |path: &str| info.as_ref().is_some_and(|i| i.links.iter().any(|(l, _, _)| l == path));

        let shared: Vec<Place> = spaces
            .windows
            .iter()
            .map(|r| {
                let unc = is_unc(&r.dir);
                let linked = info.as_ref().is_some_and(|i| i.links.iter().any(|(_, id, _)| *id == r.id));
                Place {
                    id: r.id.clone(),
                    label: r.label.clone(),
                    location: Location::Windows { root: r.id.clone(), rel: Vec::new() },
                    detail: Some(r.dir.display().to_string()),
                    linked,
                    unc,
                    warning: unc.then(|| "A network location: it can be slow, and it can't be linked into Linux.".to_string()),
                    // Probing a dead network share can hang, so only local folders are probed.
                    exists: unc || r.dir.is_dir(),
                }
            })
            .collect();

        let mut folders = Vec::new();
        let mut home = None;
        let mut roots = Vec::new();
        if let Some(i) = &info {
            home = Some(Place {
                id: "home".into(),
                label: "Home".into(),
                location: Location::Linux { path: i.home.clone() },
                detail: Some(i.home.clone()),
                linked: false,
                unc: false,
                warning: None,
                exists: true,
            });
            roots.push(Place {
                id: "root".into(),
                label: "File System".into(),
                location: Location::Linux { path: "/".into() },
                detail: Some("/".into()),
                linked: false,
                unc: false,
                warning: None,
                exists: true,
            });
            for f in SIDEBAR_ORDER {
                let path = format!("{}/{}", i.home.trim_end_matches('/'), f.linux_dir());
                let linked = linked_link(&path);
                let win = spaces.root(f.id());
                folders.push(Place {
                    id: f.id().into(),
                    label: f.label().into(),
                    location: Location::Linux { path },
                    detail: win.filter(|_| linked).map(|r| format!("Shared with Windows: {}", r.dir.display())),
                    linked,
                    unc: false,
                    warning: None,
                    exists: true,
                });
            }
        } else {
            for f in SIDEBAR_ORDER {
                if let Some(r) = spaces.root(f.id()) {
                    folders.push(Place {
                        id: f.id().into(),
                        label: f.label().into(),
                        location: Location::Windows { root: f.id().into(), rel: Vec::new() },
                        detail: Some(r.dir.display().to_string()),
                        linked: false,
                        unc: is_unc(&r.dir),
                        warning: None,
                        exists: true,
                    });
                }
            }
        }
        Ok(Places { distro, linux_available: info.is_some(), linux_note: note, home, roots, folders, shared })
    })
    .await
}

// ---------------------------------------------------------------------------------------------
// Listing
// ---------------------------------------------------------------------------------------------

#[tauri::command]
pub async fn fs_list_open(
    fs: Fs<'_>,
    cfg: Cfg<'_>,
    distro: Option<String>,
    location: Location,
    options: Option<ListOptions>,
    page_size: Option<usize>,
) -> Result<ListingHead, FsError> {
    let fs = fs.inner().clone();
    let cfg = storage_of(&cfg)?;
    run(move || {
        let spaces = fs.spaces(distro.as_deref(), &cfg, &[&location])?;
        fs.lists.open_dir(
            &spaces,
            &location,
            &options.unwrap_or_default(),
            page_size.unwrap_or(200),
        )
    })
    .await
}

#[tauri::command]
pub async fn fs_list_page(
    fs: Fs<'_>,
    id: String,
    offset: usize,
    limit: Option<usize>,
) -> Result<Page, FsError> {
    let fs = fs.inner().clone();
    run(move || fs.lists.page(&id, offset, limit.unwrap_or(200))).await
}

#[tauri::command]
pub async fn fs_list_close(fs: Fs<'_>, id: String) -> Result<(), FsError> {
    fs.lists.close(&id);
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreateKind {
    Folder,
    File,
}

#[tauri::command]
pub async fn fs_create(
    fs: Fs<'_>,
    cfg: Cfg<'_>,
    distro: Option<String>,
    parent: Location,
    name: String,
    kind: CreateKind,
) -> Result<Location, FsError> {
    let fs = fs.inner().clone();
    let cfg = storage_of(&cfg)?;
    run(move || {
        let spaces = fs.spaces(distro.as_deref(), &cfg, &[&parent])?;
        match kind {
            CreateKind::Folder => ops::create_dir(&spaces, &parent, &name),
            CreateKind::File => ops::create_file(&spaces, &parent, &name),
        }
    })
    .await
}

#[tauri::command]
pub async fn fs_rename(
    fs: Fs<'_>,
    cfg: Cfg<'_>,
    distro: Option<String>,
    location: Location,
    new_name: String,
) -> Result<Location, FsError> {
    let fs = fs.inner().clone();
    let cfg = storage_of(&cfg)?;
    run(move || {
        let spaces = fs.spaces(distro.as_deref(), &cfg, &[&location])?;
        ops::rename(&spaces, &location, &new_name)
    })
    .await
}

/// Delete as a job. `confirmed` must be true: the UI shows a confirmation first, and the backend
/// refuses without it. Links are removed, never followed.
#[tauri::command]
pub async fn fs_delete(
    fs: Fs<'_>,
    cfg: Cfg<'_>,
    distro: Option<String>,
    targets: Vec<Location>,
    confirmed: bool,
) -> Result<JobStarted, FsError> {
    let fs = fs.inner().clone();
    let cfg = storage_of(&cfg)?;
    run(move || {
        if !confirmed {
            return Err(FsError::new(
                FsErrorKind::Refused,
                "Deleting needs your confirmation. Nothing was deleted.",
            ));
        }
        let refs: Vec<&Location> = targets.iter().collect();
        let spaces = fs.spaces(distro.as_deref(), &cfg, &refs)?;
        fs.start_job("delete", move |run| {
            ops::delete(&spaces, &targets, true, run)
        })
    })
    .await
}

/// Copy or move as a job. There is no overwrite: name clashes keep both (or skip, or fail).
#[tauri::command]
pub async fn fs_transfer(
    fs: Fs<'_>,
    cfg: Cfg<'_>,
    distro: Option<String>,
    sources: Vec<Location>,
    dest: Location,
    mode: TransferMode,
    conflict: Option<Conflict>,
) -> Result<JobStarted, FsError> {
    let fs = fs.inner().clone();
    let cfg = storage_of(&cfg)?;
    run(move || {
        let mut refs: Vec<&Location> = sources.iter().collect();
        refs.push(&dest);
        let spaces = fs.spaces(distro.as_deref(), &cfg, &refs)?;
        let conflict = conflict.unwrap_or_default();
        let kind = match mode {
            TransferMode::Copy => "copy",
            TransferMode::Move => "move",
        };
        fs.start_job(kind, move |run| {
            ops::transfer(&spaces, &sources, &dest, mode, conflict, run)
        })
    })
    .await
}

#[tauri::command]
pub async fn fs_job_cancel(fs: Fs<'_>, job_id: String) -> Result<bool, FsError> {
    let g = fs
        .jobs
        .lock()
        .map_err(|_| FsError::new(FsErrorKind::Io, "internal state error"))?;
    match g.get(&job_id) {
        Some(c) => {
            c.cancel();
            Ok(true)
        }
        None => Ok(false),
    }
}

// ---------------------------------------------------------------------------------------------
// Opening in Windows
// ---------------------------------------------------------------------------------------------

fn explorer() -> PathBuf {
    match std::env::var_os("SystemRoot") {
        Some(root) => PathBuf::from(root).join("explorer.exe"),
        None => PathBuf::from("explorer.exe"),
    }
}

#[cfg(windows)]
fn launch(path: &Path, select: bool) -> FsResult<()> {
    use std::os::windows::process::CommandExt;
    let text = path.display().to_string();
    if text.contains('"') {
        return Err(FsError::new(
            FsErrorKind::InvalidPath,
            "That path contains characters that can't be passed to Windows safely.",
        ));
    }
    let mut cmd = std::process::Command::new(explorer());
    if select {
        cmd.raw_arg(format!("/select,\"{text}\""));
    } else {
        cmd.arg(path);
    }
    // explorer.exe reports failure through its exit code unreliably, so only launching is checked.
    cmd.spawn().map(|_| ()).map_err(|e| {
        FsError::new(
            FsErrorKind::Io,
            format!("Couldn't start Windows Explorer: {e}"),
        )
    })
}

#[cfg(not(windows))]
fn launch(_path: &Path, _select: bool) -> FsResult<()> {
    let _ = explorer();
    Err(unavailable(
        "Opening files in Windows is only available on Windows.",
    ))
}

/// Open a file with its Windows default application. Programs and scripts are refused, folders
/// open inside Files instead.
#[tauri::command]
pub async fn fs_open(
    fs: Fs<'_>,
    cfg: Cfg<'_>,
    distro: Option<String>,
    location: Location,
) -> Result<(), FsError> {
    let fs = fs.inner().clone();
    let cfg = storage_of(&cfg)?;
    run(move || {
        let spaces = fs.spaces(distro.as_deref(), &cfg, &[&location])?;
        let entry = spaces.resolve_entry(&location)?;
        let policy = open_policy(&entry.name);
        if !policy.allowed {
            return Err(FsError::new(
                FsErrorKind::Refused,
                policy
                    .reason
                    .unwrap_or_else(|| "This type of file is not opened from VLINUX.".into()),
            ));
        }
        let meta = std::fs::metadata(&entry.host)
            .map_err(|e| FsError::from_io(&e, "open", &entry.name))?;
        if meta.is_dir() {
            return Err(FsError::new(
                FsErrorKind::IsADirectory,
                "Folders open inside Files.",
            ));
        }
        launch(&entry.host, false)
    })
    .await
}

/// Show an item (or a folder) in Windows Explorer.
#[tauri::command]
pub async fn fs_reveal(
    fs: Fs<'_>,
    cfg: Cfg<'_>,
    distro: Option<String>,
    location: Location,
) -> Result<(), FsError> {
    let fs = fs.inner().clone();
    let cfg = storage_of(&cfg)?;
    run(move || {
        let spaces = fs.spaces(distro.as_deref(), &cfg, &[&location])?;
        match spaces.resolve_entry(&location) {
            Ok(entry) => {
                let is_dir = std::fs::metadata(&entry.host)
                    .map(|m| m.is_dir())
                    .unwrap_or(false);
                launch(&entry.host, !is_dir)
            }
            // A shared folder or the Linux root itself has no parent entry: open it as a folder.
            Err(e) if e.kind == FsErrorKind::Refused => {
                let (_, host) = spaces.resolve_dir(&location)?;
                launch(&host, false)
            }
            Err(e) => Err(e),
        }
    })
    .await
}

// ---------------------------------------------------------------------------------------------
// Mappings
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct MappingReport {
    distro: String,
    /// True when the live state of the distribution was inspected for this report.
    checked: bool,
    home: Option<String>,
    mount_root: String,
    automount: bool,
    folders: Vec<MappingStatus>,
}

#[derive(Debug, Serialize)]
pub struct MappingOutcome {
    message: String,
    status: MappingStatus,
}

impl FilesState {
    fn with_ctx<T>(
        &self,
        distro: &str,
        info: Option<&DistroInfo>,
        f: impl FnOnce(&Ctx<'_>) -> T,
    ) -> FsResult<T> {
        let ops = self.distro_ops(distro)?;
        let resolver = SystemKnownFolders;
        let drives = SystemDrives;
        let exists = |p: &str| Path::new(p).is_dir();
        let root = info.map(|i| i.root.clone()).unwrap_or_default();
        let (home, mount_root, automount) = match info {
            Some(i) => (
                i.home.as_str(),
                i.conf.mount_root.as_str(),
                i.conf.automount_enabled,
            ),
            None => ("", "/mnt", true),
        };
        let ctx = Ctx {
            distro,
            linux_root: &root,
            home,
            mount_root,
            automount,
            ops: &ops,
            drives: &drives,
            resolver: &resolver,
            windows_dir_exists: &exists,
            now: unix_now(),
        };
        Ok(f(&ctx))
    }

    fn report(&self, distro: &str, cfg: &StorageConfig, check: bool) -> FsResult<MappingReport> {
        let info = if check {
            Some(self.info(distro, true)?)
        } else {
            self.cached_info(distro)
        };
        let folders = self.with_ctx(distro, info.as_deref(), |ctx| {
            KnownFolder::ALL
                .iter()
                .map(|&f| engine::status(ctx, &self.store, f, cfg.folder(f), check))
                .collect::<Vec<_>>()
        })?;
        Ok(MappingReport {
            distro: distro.to_string(),
            checked: check,
            home: info.as_ref().map(|i| i.home.clone()),
            mount_root: info
                .as_ref()
                .map(|i| i.conf.mount_root.clone())
                .unwrap_or_else(|| "/mnt".into()),
            automount: info
                .as_ref()
                .map(|i| i.conf.automount_enabled)
                .unwrap_or(true),
            folders,
        })
    }
}

fn check_distro_name(distro: &str) -> FsResult<()> {
    if valid_distro_name(distro) {
        Ok(())
    } else {
        Err(FsError::new(
            FsErrorKind::InvalidPath,
            format!("'{distro}' is not a valid distribution name."),
        ))
    }
}

/// Where each folder stands. Without `check` no Linux process is started and nothing is claimed as
/// connected; with `check` the distribution is inspected (and started if needed).
#[tauri::command]
pub async fn mapping_status(
    fs: Fs<'_>,
    cfg: Cfg<'_>,
    distro: String,
    check: Option<bool>,
) -> Result<MappingReport, FsError> {
    let fs = fs.inner().clone();
    let cfg = storage_of(&cfg)?;
    run(move || {
        check_distro_name(&distro)?;
        let _gate = fs.gate.lock().unwrap_or_else(|p| p.into_inner());
        fs.report(&distro, &cfg, check.unwrap_or(false))
    })
    .await
}

fn one_status(
    fs: &FilesState,
    distro: &str,
    cfg: &StorageConfig,
    folder: KnownFolder,
) -> FsResult<MappingStatus> {
    let info = fs.info(distro, true)?;
    fs.with_ctx(distro, Some(&*info), |ctx| {
        engine::status(ctx, &fs.store, folder, cfg.folder(folder), true)
    })
}

/// Apply the saved preference of one folder. `resolution` is the user's explicit choice when the
/// status asked for one; without it nothing that exists is changed.
#[tauri::command]
pub async fn mapping_apply(
    fs: Fs<'_>,
    cfg: Cfg<'_>,
    distro: String,
    folder: KnownFolder,
    resolution: Option<Resolution>,
) -> Result<MappingOutcome, FsError> {
    let fs = fs.inner().clone();
    let cfg = storage_of(&cfg)?;
    run(move || {
        check_distro_name(&distro)?;
        let _gate = fs.gate.lock().unwrap_or_else(|p| p.into_inner());
        let info = fs.info(&distro, true)?;
        let result = fs.with_ctx(&distro, Some(&*info), |ctx| {
            engine::apply(ctx, &fs.store, folder, cfg.folder(folder), resolution)
        })?;
        fs.forget(&distro);
        let message = result.map_err(|m| FsError::new(FsErrorKind::Refused, m))?;
        let status = one_status(&fs, &distro, &cfg, folder)?;
        Ok(MappingOutcome { message, status })
    })
    .await
}

/// Undo one folder's link and restore what was there before. Deletes nothing.
#[tauri::command]
pub async fn mapping_undo(
    fs: Fs<'_>,
    cfg: Cfg<'_>,
    distro: String,
    folder: KnownFolder,
) -> Result<MappingOutcome, FsError> {
    let fs = fs.inner().clone();
    let cfg = storage_of(&cfg)?;
    run(move || {
        check_distro_name(&distro)?;
        let _gate = fs.gate.lock().unwrap_or_else(|p| p.into_inner());
        let info = fs.info(&distro, true)?;
        let result = fs.with_ctx(&distro, Some(&*info), |ctx| {
            engine::undo(ctx, &fs.store, folder, cfg.folder(folder))
        })?;
        fs.forget(&distro);
        let message = result.map_err(|m| FsError::new(FsErrorKind::Refused, m))?;
        let status = one_status(&fs, &distro, &cfg, folder)?;
        Ok(MappingOutcome { message, status })
    })
    .await
}
