//! Read-only WSL detection.
//!
//! What runs: `wsl.exe --status`, `--version`, `--list --verbose`, and — only for distributions
//! that are *already running* — a read-only `cat /etc/os-release`. Stopped distributions are never
//! started just to identify them (they are identified from their registered name; the user can
//! opt in to a precise identification later). When WSL is not operational, two `reg.exe query`
//! probes look for a pending Windows restart. Nothing here changes any state.

use std::collections::HashSet;

use crate::adapters::DistroRegistry;
use crate::encoding::decode_wsl_output;
use crate::exec::{CmdOutput, CommandSpec, RunError};
use crate::model::*;
use crate::osrelease::parse_os_release;
use crate::parse::{self, Failure};
use crate::unix_now;
use crate::wsl::{Programs, WslClient};

/// WSL 2 needs Windows 10 version 1903 (build 18362) or later.
pub const MIN_WSL2_BUILD: u32 = 18362;
/// Upper bound on `/etc/os-release` reads per detection.
pub const MAX_OS_RELEASE_QUERIES: usize = 8;

pub struct DetectOptions<'a> {
    pub host: HostEnv,
    /// Registered names VLINUX itself installed (compared case-insensitively).
    pub managed: &'a HashSet<String>,
    /// Read `/etc/os-release` from already-running distributions (never starts a distribution).
    pub identify_running: bool,
}

const REBOOT_KEYS: [&str; 2] = [
    r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending",
    r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update\RequiredReboot",
];

/// Heuristic: do the standard Windows servicing "restart pending" registry keys exist?
pub fn probe_pending_reboot(wsl: &WslClient, programs: &Programs) -> bool {
    REBOOT_KEYS.iter().any(|key| {
        let spec = CommandSpec::new(programs.reg.clone(), &["query", *key], std::time::Duration::from_secs(5));
        matches!(wsl.runner().run_simple(&spec), Ok(o) if o.success())
    })
}

pub fn is_internal_distro(name: &str) -> bool {
    let lc = name.to_ascii_lowercase();
    ["docker-desktop", "rancher-desktop", "podman-machine"].iter().any(|p| lc.starts_with(p))
}

pub(crate) fn first_line(text: &str) -> String {
    let line = text.lines().map(|l| l.trim()).find(|l| !l.is_empty()).unwrap_or("");
    line.chars().take(240).collect()
}

fn text_of(out: &CmdOutput) -> String {
    let mut t = decode_wsl_output(&out.stdout);
    let e = decode_wsl_output(&out.stderr);
    if !e.is_empty() {
        t.push('\n');
        t.push_str(&e);
    }
    t
}

struct Builder {
    report: WslReport,
}

impl Builder {
    fn new(host: HostEnv) -> Self {
        Self {
            report: WslReport {
                status: RuntimeStatus::NotInstalled,
                host,
                wsl_version: None,
                kernel_version: None,
                wslg_version: None,
                default_wsl_version: None,
                wsl2_available: None,
                distros: Vec::new(),
                default_distro: None,
                suggested_distro: None,
                pending_reboot: None,
                warnings: Vec::new(),
                detected_at: unix_now(),
            },
        }
    }
    fn done(mut self, status: RuntimeStatus) -> WslReport {
        self.report.status = status;
        self.report
    }
}

fn run_error_message(what: &str, e: &RunError) -> String {
    match e {
        RunError::Timeout(d) => format!("{what} did not answer within {} s. WSL may be starting up or stuck; try again.", d.as_secs()),
        other => format!("{what} failed: {other}."),
    }
}

/// Turn a recognised or unrecognised failure into a status, consulting the pending-restart probe
/// where that is meaningful.
fn failure_status(
    b: &mut Builder,
    wsl: &WslClient,
    programs: &Programs,
    failure: Option<Failure>,
    what: &str,
    text: &str,
    code: Option<i32>,
) -> RuntimeStatus {
    let pending = probe_pending_reboot(wsl, programs);
    b.report.pending_reboot = Some(pending);
    match failure {
        Some(Failure::NotInstalled) => {
            if pending {
                RuntimeStatus::RestartRequired
            } else {
                RuntimeStatus::NotInstalled
            }
        }
        Some(Failure::Code { code, hint }) => {
            if pending {
                RuntimeStatus::RestartRequired
            } else {
                RuntimeStatus::FeatureProblem { code: Some(code.to_string()), hint: hint.to_string() }
            }
        }
        Some(Failure::NoDistributions) | None => {
            if pending {
                b.report.warnings.push("Windows reports a pending restart; restarting may resolve this.".to_string());
            }
            let msg = first_line(text);
            RuntimeStatus::Error {
                message: if msg.is_empty() {
                    format!("{what} failed (exit code {}).", code.map(|c| c.to_string()).unwrap_or_else(|| "unknown".into()))
                } else {
                    format!("{what} failed: {msg}")
                },
            }
        }
    }
}

pub fn detect(wsl: &WslClient, programs: &Programs, registry: &DistroRegistry, opts: &DetectOptions<'_>) -> WslReport {
    let mut b = Builder::new(opts.host.clone());

    // 1. Host support.
    if !opts.host.is_windows {
        return b.done(RuntimeStatus::UnsupportedHost {
            reason: "The WSL backend only runs on Windows. This build is running on another operating system.".to_string(),
        });
    }
    if let Some(build) = opts.host.windows_build {
        if build < MIN_WSL2_BUILD {
            b.report.wsl2_available = Some(false);
            return b.done(RuntimeStatus::UnsupportedHost {
                reason: format!("WSL 2 needs Windows 10 version 1903 (build {MIN_WSL2_BUILD}) or later; this PC is on build {build}. Update Windows first."),
            });
        }
    }

    // 2. `wsl.exe --status`
    let mut no_distros_hint = false;
    match wsl.status() {
        Err(RunError::NotFound(_)) => {
            let status = failure_status(&mut b, wsl, programs, Some(Failure::NotInstalled), "wsl.exe", "", None);
            return b.done(status);
        }
        Err(e) => {
            let msg = run_error_message("'wsl.exe --status'", &e);
            return b.done(RuntimeStatus::Error { message: msg });
        }
        Ok(out) => {
            let text = text_of(&out);
            if out.success() {
                let info = parse::parse_status(&text);
                b.report.default_wsl_version = info.default_version;
                // The default distribution is taken from the '*' marker below; keep the status
                // value only as a fallback.
                if let Some(n) = info.default_distro {
                    b.report.default_distro = Some(n);
                }
            } else {
                match parse::classify_failure(&text) {
                    Some(Failure::NoDistributions) => no_distros_hint = true,
                    other => {
                        let status = failure_status(&mut b, wsl, programs, other, "'wsl.exe --status'", &text, out.status);
                        return b.done(status);
                    }
                }
            }
        }
    }

    // 3. `wsl.exe --version` (optional; not available on very old inbox WSL).
    if let Ok(out) = wsl.version() {
        if out.success() {
            let v = parse::parse_version(&text_of(&out));
            b.report.wsl_version = v.wsl;
            b.report.kernel_version = v.kernel;
            b.report.wslg_version = v.wslg;
        }
    }

    // 4. `wsl.exe --list --verbose`
    let list_out = match wsl.list_verbose() {
        Ok(o) => o,
        Err(e) => {
            let msg = run_error_message("'wsl.exe --list --verbose'", &e);
            return b.done(RuntimeStatus::Error { message: msg });
        }
    };
    let list_text = text_of(&list_out);
    let (rows, unparsed) = parse::parse_list_verbose(&list_text);

    if rows.is_empty() {
        let failure = parse::classify_failure(&list_text);
        return match failure {
            Some(Failure::NoDistributions) => b.done(RuntimeStatus::NoDistributions),
            _ if no_distros_hint && list_text.trim().is_empty() => b.done(RuntimeStatus::NoDistributions),
            Some(f) => {
                let status = failure_status(&mut b, wsl, programs, Some(f), "'wsl.exe --list --verbose'", &list_text, list_out.status);
                b.done(status)
            }
            None => {
                let line = first_line(&list_text);
                let message = if line.is_empty() {
                    "'wsl.exe --list --verbose' returned no output.".to_string()
                } else {
                    format!("VLINUX could not understand the output of 'wsl.exe --list --verbose' (\"{line}\"). WSL's output format may have changed.")
                };
                b.done(RuntimeStatus::Error { message })
            }
        };
    }

    let surprising = unparsed
        .iter()
        .filter(|l| !(l.split_whitespace().count() >= 2 && !l.chars().any(|c| c.is_lowercase() || c.is_ascii_digit())))
        .count();
    if surprising > 0 {
        b.report.warnings.push(format!("Ignored {surprising} line(s) of 'wsl.exe --list --verbose' output that VLINUX did not recognise."));
    }

    // 5. Build distributions.
    let managed: HashSet<String> = opts.managed.iter().map(|n| n.to_ascii_lowercase()).collect();
    let mut queried = 0usize;
    let mut distros = Vec::new();
    for row in &rows {
        let kind = if is_internal_distro(&row.name) { DistroKind::Internal } else { DistroKind::Regular };
        let by_name = registry.from_registered_name(&row.name);
        let mut adapter = by_name.info();
        let mut identity_source = IdentitySource::RegisteredName;
        let mut os_release = None;

        if opts.identify_running && kind == DistroKind::Regular && row.state == DistroState::Running {
            if queried < MAX_OS_RELEASE_QUERIES {
                queried += 1;
                match wsl.read_os_release(&row.name) {
                    Ok(o) if o.success() => {
                        let parsed = parse_os_release(&decode_wsl_output(&o.stdout));
                        adapter = registry.from_os_release(&parsed.id, &parsed.id_like).adapter.info();
                        identity_source = IdentitySource::OsRelease;
                        os_release = Some(parsed);
                    }
                    Ok(o) => b.report.warnings.push(format!(
                        "Could not read /etc/os-release in '{}' (exit code {}); using its registered name instead.",
                        row.name,
                        o.status.map(|c| c.to_string()).unwrap_or_else(|| "?".into())
                    )),
                    Err(e) => b.report.warnings.push(format!("Could not identify '{}': {e}. Using its registered name instead.", row.name)),
                }
            }
        }

        let display_label = os_release
            .as_ref()
            .and_then(|r| r.pretty_name.clone().or_else(|| r.name.clone()))
            .unwrap_or_else(|| if adapter.id == "generic" { row.name.clone() } else { adapter.display_name.clone() });

        distros.push(Distro {
            name: row.name.clone(),
            state: row.state,
            wsl_version: row.version,
            is_default: row.is_default,
            kind,
            origin: if managed.contains(&row.name.to_ascii_lowercase()) { Origin::InstalledByVlinux } else { Origin::Existing },
            adapter,
            identity_source,
            os_release,
            display_label,
        });
    }

    // Default distribution: the '*' marker is authoritative (language independent).
    if let Some(d) = distros.iter().find(|d| d.is_default) {
        b.report.default_distro = Some(d.name.clone());
    } else if let Some(n) = b.report.default_distro.clone() {
        if !distros.iter().any(|d| d.name.eq_ignore_ascii_case(&n)) {
            b.report.default_distro = None;
        }
    }

    // Suggestion: the best-ranked existing WSL 2 distribution. Never selected automatically.
    let rank = |d: &Distro| -> u8 {
        registry
            .by_id(&d.adapter.id)
            .map(|a| a.existing_install_rank())
            .unwrap_or_else(|| registry.generic().existing_install_rank())
    };
    b.report.suggested_distro = distros
        .iter()
        .filter(|d| d.kind == DistroKind::Regular && d.wsl_version == 2 && matches!(d.state, DistroState::Running | DistroState::Stopped))
        .min_by_key(|d| (rank(d), !d.is_default))
        .map(|d| d.name.clone());

    let any_v2 = distros.iter().any(|d| d.wsl_version == 2);
    if b.report.kernel_version.is_some() || any_v2 {
        b.report.wsl2_available = Some(true);
    }

    let usable = distros.iter().any(|d| d.kind == DistroKind::Regular);
    b.report.distros = distros;
    let status = if usable { RuntimeStatus::Ready } else { RuntimeStatus::NoDistributions };
    b.done(status)
}

/// Fetch the official install list (`wsl.exe --list --online`; needs internet) and match it
/// against the adapter registry.
pub fn fetch_catalog(wsl: &WslClient, registry: &DistroRegistry, existing: &[Distro]) -> Catalog {
    let fetched_at = unix_now();
    let fail = |msg: String| Catalog { entries: Vec::new(), error: Some(msg), fetched_at };
    let out = match wsl.list_online() {
        Ok(o) => o,
        Err(e) => return fail(format!("Could not fetch the official distribution list: {e}. This needs an internet connection.")),
    };
    let text = text_of(&out);
    let rows = parse::parse_online_list(&text);
    if rows.is_empty() {
        let line = first_line(&text);
        return fail(if line.is_empty() {
            "The official distribution list was empty.".to_string()
        } else {
            format!("Could not fetch the official distribution list: {line}")
        });
    }
    let mut entries: Vec<CatalogEntry> = rows
        .into_iter()
        .map(|r| {
            let adapter = registry.from_wsl_online_name(&r.name).map(|a| a.info());
            let already_installed = existing.iter().any(|d| d.name.eq_ignore_ascii_case(&r.name));
            CatalogEntry { supported: adapter.is_some(), adapter, wsl_name: r.name, friendly_name: r.friendly_name, already_installed }
        })
        .collect();
    // Supported distributions first (stable), keeping Microsoft's order within each group.
    entries.sort_by_key(|e| !e.supported);
    Catalog { entries, error: None, fetched_at }
}
