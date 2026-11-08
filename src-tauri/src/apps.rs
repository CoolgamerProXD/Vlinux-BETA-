//! Tauri bindings for the launcher's Linux applications.
//!
//! * Discovery only **reads** Desktop Entry files through the distribution's file share, and only
//!   when the distribution is already running — reading the share of a stopped distribution would
//!   start it, and VLINUX never starts a distribution without an explicit user action.
//! * The frontend sends a distribution name and a desktop-file *id*. The backend re-reads the entry,
//!   builds an argument vector and runs `wsl.exe -d <name> --exec <argv...>` directly (no shell).
//! * Linux GUI applications are shown by WSLg as separate, Windows-managed windows. They are not
//!   embedded in the VLINUX shell, and closing VLINUX does not close them.
//! * Progress is pushed as `vlinux://app-launch` events; nothing polls.

use std::collections::HashMap;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use vlinux_runtime::apps::{self, LinuxApp};
use vlinux_runtime::exec::SystemRunner;
use vlinux_runtime::model::DistroState;
use vlinux_runtime::parse::valid_distro_name;
use vlinux_runtime::shared::{DistroOps, WslDistroOps};
use vlinux_runtime::terminal::b64;

use crate::error::AppError;
use crate::files::linux_root;
use crate::runtime::{blocking, ensure_wsl_ready, require_regular_distro, RuntimeState};

pub const LAUNCH_EVENT: &str = "vlinux://app-launch";
const MAX_ICON_REQUEST: usize = 80;
const MAX_RECORDS: usize = 50;
const STDERR_CAP: usize = 4096;

type Rt<'a> = State<'a, Arc<RuntimeState>>;
type Apps<'a> = State<'a, Arc<AppsState>>;

#[derive(Default)]
struct DistroCache {
    home: Option<String>,
    /// desktop-file id → `Icon=` value
    icons: HashMap<String, String>,
}

#[derive(Serialize, Clone)]
pub struct LaunchRecord {
    pub launch_id: u64,
    pub id: String,
    pub name: String,
    /// `running`, `exited` or `failed`.
    pub state: String,
    pub code: Option<i32>,
    pub message: Option<String>,
}

pub struct AppsState {
    app: AppHandle,
    rt: Arc<RuntimeState>,
    cache: Mutex<HashMap<String, DistroCache>>,
    records: Mutex<Vec<LaunchRecord>>,
    counter: AtomicU64,
}

impl AppsState {
    pub fn new(app: AppHandle, rt: Arc<RuntimeState>) -> Self {
        Self {
            app,
            rt,
            cache: Mutex::new(HashMap::new()),
            records: Mutex::new(Vec::new()),
            counter: AtomicU64::new(1),
        }
    }

    fn publish(&self, rec: LaunchRecord) {
        if let Ok(mut g) = self.records.lock() {
            if let Some(slot) = g.iter_mut().find(|r| r.launch_id == rec.launch_id) {
                *slot = rec.clone();
            } else {
                g.push(rec.clone());
                let extra = g.len().saturating_sub(MAX_RECORDS);
                if extra > 0 {
                    g.drain(..extra);
                }
            }
        }
        let _ = self.app.emit(LAUNCH_EVENT, rec);
    }
}

#[derive(Serialize)]
pub struct AppsReport {
    pub distro: String,
    /// `ready`, `stopped` or `unavailable`.
    pub state: String,
    pub message: Option<String>,
    /// Whether this distribution has WSLg's runtime folder (`/mnt/wslg`), which Linux GUI apps need.
    pub wslg: bool,
    pub apps: Vec<LinuxApp>,
    pub folders: Vec<String>,
    pub skipped: usize,
    pub warnings: Vec<String>,
    pub elapsed_ms: u64,
}

fn empty_report(distro: &str, state: &str, message: String) -> AppsReport {
    AppsReport {
        distro: distro.to_string(),
        state: state.to_string(),
        message: Some(message),
        wslg: false,
        apps: Vec::new(),
        folders: Vec::new(),
        skipped: 0,
        warnings: Vec::new(),
        elapsed_ms: 0,
    }
}

/// Resolve the share root and home folder of a **running** distribution.
/// `Err(report)` carries the not-ready answer for the UI.
fn running_context(
    st: &AppsState,
    distro: &str,
) -> Result<(std::path::PathBuf, Option<String>), Box<AppsReport>> {
    if !valid_distro_name(distro) {
        return Err(Box::new(empty_report(
            distro,
            "unavailable",
            format!("'{distro}' is not a valid distribution name."),
        )));
    }
    // `wsl -l -v` reports state without starting anything; force a fresh answer because the
    // distribution may have stopped since the last check.
    let report = st.rt.detect(true);
    if let Err(e) = ensure_wsl_ready(&report).and_then(|()| require_regular_distro(&report, distro)) {
        return Err(Box::new(empty_report(distro, "unavailable", e.to_string())));
    }
    let running = report
        .distros
        .iter()
        .any(|d| d.name == distro && d.state == DistroState::Running);
    if !running {
        return Err(Box::new(empty_report(
            distro,
            "stopped",
            format!("'{distro}' is not running. Start it to see and launch its applications."),
        )));
    }
    let root = linux_root(distro)
        .map_err(|e| Box::new(empty_report(distro, "unavailable", e.message)))?;
    let home = WslDistroOps::new(Arc::new(SystemRunner), st.rt.programs.wsl.clone(), distro)
        .ok()
        .and_then(|ops| ops.home_dir().ok());
    Ok((root, home))
}

/// The applications installed in a running distribution.
#[tauri::command]
pub async fn apps_discover(st: Apps<'_>, distro: String) -> Result<AppsReport, AppError> {
    let st = st.inner().clone();
    blocking(move || {
        let t0 = Instant::now();
        let (root, home) = match running_context(&st, &distro) {
            Ok(v) => v,
            Err(r) => return Ok(*r),
        };
        let scan = apps::scan(&root, home.as_deref());
        let wslg = root.join("mnt").join("wslg").is_dir();
        if let Ok(mut g) = st.cache.lock() {
            g.insert(
                distro.clone(),
                DistroCache {
                    home: home.clone(),
                    icons: scan
                        .found
                        .iter()
                        .filter_map(|f| f.app.icon.clone().map(|i| (f.app.id.clone(), i)))
                        .collect(),
                },
            );
        }
        let mut warnings = scan.warnings.clone();
        if home.is_none() {
            warnings.push("The home folder could not be read, so per-user applications are not listed.".into());
        }
        let message = scan.found.is_empty().then(|| {
            "No desktop applications are installed in this distribution. Install some with its package manager (Linux Setup can search packages).".to_string()
        });
        Ok(AppsReport {
            distro,
            state: "ready".into(),
            message,
            wslg,
            apps: scan.found.into_iter().map(|f| f.app).collect(),
            folders: scan.scanned,
            skipped: scan.skipped,
            warnings,
            elapsed_ms: u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX),
        })
    })
    .await
}

/// Icons for up to 80 application ids from the last discovery, as `data:` URLs. Ids without a
/// usable icon are simply absent, so the UI draws its own tile.
#[tauri::command]
pub async fn apps_icons(
    st: Apps<'_>,
    distro: String,
    ids: Vec<String>,
) -> Result<HashMap<String, String>, AppError> {
    let st = st.inner().clone();
    blocking(move || {
        if !valid_distro_name(&distro) || ids.len() > MAX_ICON_REQUEST {
            return Err(AppError::Runtime("Invalid icon request.".into()));
        }
        let (home, wanted): (Option<String>, Vec<(String, String)>) = {
            let g = st
                .cache
                .lock()
                .map_err(|_| AppError::Runtime("app cache lock poisoned".into()))?;
            let Some(c) = g.get(&distro) else {
                return Ok(HashMap::new());
            };
            (
                c.home.clone(),
                ids.iter()
                    .filter(|i| apps::valid_app_id(i))
                    .filter_map(|i| c.icons.get(i).map(|n| (i.clone(), n.clone())))
                    .collect(),
            )
        };
        let Ok(root) = linux_root(&distro) else {
            return Ok(HashMap::new());
        };
        let mut out = HashMap::new();
        for (id, name) in wanted {
            if let Some((mime, bytes)) = apps::find_icon(&root, home.as_deref(), &name) {
                out.insert(id, format!("data:{mime};base64,{}", b64::encode(&bytes)));
            }
        }
        Ok(out)
    })
    .await
}

fn clean_output(raw: &[u8]) -> String {
    let no_nul: Vec<u8> = raw.iter().copied().filter(|b| *b != 0).collect();
    let text = String::from_utf8_lossy(&no_nul);
    let t: String = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take(6)
        .collect::<Vec<_>>()
        .join(" · ");
    t.chars().take(500).collect()
}

/// Launch one application of a running distribution. Returns as soon as the process has started;
/// the outcome follows as `vlinux://app-launch` events.
#[tauri::command]
pub async fn apps_launch(
    st: Apps<'_>,
    distro: String,
    id: String,
) -> Result<LaunchRecord, AppError> {
    let st = st.inner().clone();
    blocking(move || {
        if !apps::valid_app_id(&id) {
            return Err(AppError::Runtime("That is not a valid application id.".into()));
        }
        let (root, home) = match running_context(&st, &distro) {
            Ok(v) => v,
            Err(r) => return Err(AppError::Runtime(r.message.unwrap_or_default())),
        };
        let (app, argv) =
            apps::launch_argv(&root, home.as_deref(), &id).map_err(AppError::Runtime)?;

        let mut cmd = Command::new(&st.rt.programs.wsl);
        cmd.arg("-d")
            .arg(&distro)
            .arg("--exec")
            .args(&argv)
            .env("WSL_UTF8", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| AppError::Runtime(format!("Couldn't start '{}': {e}", app.name)))?;

        let launch_id = st.counter.fetch_add(1, Ordering::SeqCst);
        let base = LaunchRecord {
            launch_id,
            id: id.clone(),
            name: app.name.clone(),
            state: "running".into(),
            code: None,
            message: None,
        };
        st.publish(base.clone());

        // Collect a bounded amount of error output, then report how the process ended.
        let stderr = child.stderr.take();
        let monitor = st.clone();
        let rec = base.clone();
        std::thread::Builder::new()
            .name(format!("app-launch-{launch_id}"))
            .spawn(move || {
                let mut buf = Vec::new();
                if let Some(mut s) = stderr {
                    let mut chunk = [0u8; 1024];
                    while let Ok(n) = s.read(&mut chunk) {
                        if n == 0 {
                            break;
                        }
                        if buf.len() < STDERR_CAP {
                            let room = STDERR_CAP - buf.len();
                            buf.extend_from_slice(&chunk[..n.min(room)]);
                        }
                    }
                }
                let status = child.wait();
                let text = clean_output(&buf);
                let mut done = rec;
                match status {
                    Ok(s) if s.success() => {
                        done.state = "exited".into();
                        done.code = s.code();
                    }
                    Ok(s) => {
                        done.state = "failed".into();
                        done.code = s.code();
                        done.message = Some(if text.is_empty() {
                            format!("It stopped with exit code {}.", s.code().map_or_else(|| "?".to_string(), |c| c.to_string()))
                        } else {
                            text
                        });
                    }
                    Err(e) => {
                        done.state = "failed".into();
                        done.message = Some(format!("Couldn't read how it ended: {e}"));
                    }
                }
                monitor.publish(done);
            })
            .map_err(|e| AppError::Runtime(format!("Couldn't watch the launch: {e}")))?;
        Ok(base)
    })
    .await
}

/// Launches seen in this session (newest last), so the UI can recover after a reload.
#[tauri::command]
pub fn apps_launched(st: Apps<'_>) -> Vec<LaunchRecord> {
    st.records.lock().map(|g| g.clone()).unwrap_or_default()
}
