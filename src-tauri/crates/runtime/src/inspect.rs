//! Read-only distribution inspection (M5).
//!
//! Inspection reads `/etc/os-release`, `uname`, `id` and probes for package managers *inside* the
//! distribution. It changes nothing. A stopped distribution is only inspected with the user's
//! explicit say-so (`allow_start`), because the first command starts it; the result always says
//! whether inspection started the distribution. Results are cached on disk with their timestamp
//! (`inspections.json`) so the desktop never needs to wake a distribution just to show facts, and a
//! refresh is always an explicit action.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::adapters::{DistroRegistry, PackageManager};
use crate::exec::{CancelToken, CommandRunner, RunError};
use crate::lifecycle::{check_target, LifecycleError};
use crate::model::{DistroState, OsRelease, WslReport};
use crate::osrelease::parse_os_release;
use crate::packages::{parse_version, Availability, PackageCaps, PackageSupport, PROBE_BINARIES};
use crate::probe::{Probed, Prober};
use crate::wsl::WslClient;

const INSPECTIONS_FILE: &str = "inspections.json";

/// What probing found for one package-manager binary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagerProbe {
    pub binary: String,
    pub present: bool,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inspection {
    pub distro: String,
    pub at: u64,
    pub was_running: bool,
    /// Inspection had to start this distribution (the user allowed it).
    pub started_by_inspection: bool,
    pub os_release: Option<OsRelease>,
    pub kernel: Option<String>,
    pub arch: Option<String>,
    /// `id -un` in the distribution: the user a terminal opens as. Not the Windows account.
    pub default_user: Option<String>,
    pub uid: Option<u32>,
    pub managers: Vec<ManagerProbe>,
    /// Things that could not be determined, in plain words.
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InspectOutcome {
    Done {
        inspection: Inspection,
    },
    /// The distribution is stopped and the caller did not allow starting it. Nothing was run.
    NeedsStart {
        distro: String,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct InspectOptions {
    pub allow_start: bool,
}

/// Which package manager VLINUX should treat this distribution as using, from what probing found.
/// Never assumed from the name: the adapter's manager counts only if its binary was found; an
/// unidentified distribution falls back to the single supported manager that probing found.
pub fn resolve_packages(
    adapter: PackageSupport,
    managers: &[ManagerProbe],
) -> (PackageSupport, Availability, Vec<String>) {
    if managers.is_empty() {
        return (adapter, Availability::NotChecked, Vec::new());
    }
    let find = |bin: &str| managers.iter().find(|m| m.binary == bin);
    let mut notes = Vec::new();
    if let Some(bin) = adapter.binary() {
        match find(bin) {
            Some(m) if m.present => {
                return (
                    adapter,
                    Availability::Present {
                        version: m.version.clone(),
                    },
                    notes,
                )
            }
            _ => {}
        }
    }
    // The adapter's own manager is missing or the distribution is unidentified: look at what *is* there.
    let present: Vec<&ManagerProbe> = managers.iter().filter(|m| m.present).collect();
    let supported: Vec<PackageSupport> = present
        .iter()
        .filter_map(|m| {
            [
                PackageManager::Apt,
                PackageManager::Pacman,
                PackageManager::Dnf,
            ]
            .iter()
            .map(|pm| PackageSupport::for_manager(*pm))
            .find(|s| s.binary() == Some(m.binary.as_str()))
        })
        .collect();
    if adapter.manager == PackageManager::Unknown && supported.len() == 1 {
        let s = supported[0];
        let version = find(s.binary().unwrap_or("")).and_then(|m| m.version.clone());
        notes.push(format!("{} was found by probing; the distribution itself is not one VLINUX has an adapter for.", s.manager.display_name()));
        return (s, Availability::Present { version }, notes);
    }
    let others: Vec<&str> = present.iter().map(|m| m.binary.as_str()).collect();
    if !others.is_empty() {
        notes.push(format!(
            "Found {} here, which VLINUX has no package adapter for.",
            others.join(", ")
        ));
    }
    (adapter, Availability::Absent, notes)
}

impl Inspection {
    pub fn package_caps(&self, adapter: PackageSupport) -> (PackageCaps, Vec<String>) {
        let (support, availability, notes) = resolve_packages(adapter, &self.managers);
        (support.capabilities(availability), notes)
    }
}

fn map_run(what: &str, e: RunError) -> LifecycleError {
    match e {
        RunError::Cancelled => {
            LifecycleError::Failed("Inspection was cancelled. Nothing was changed.".into())
        }
        RunError::Timeout(d) => {
            LifecycleError::Failed(format!("{what} did not finish within {} s.", d.as_secs()))
        }
        other => LifecycleError::Failed(format!("{what} failed: {other}.")),
    }
}

/// Inspect one distribution. The caller holds an `OpRegistry` token for it.
pub fn inspect(
    runner: &Arc<dyn CommandRunner>,
    program: &str,
    report: &WslReport,
    name: &str,
    opts: InspectOptions,
    cancel: &CancelToken,
    registry: &DistroRegistry,
    now: u64,
) -> Result<InspectOutcome, LifecycleError> {
    let distro = check_target(report, name)?;
    let was_running = distro.state == DistroState::Running;
    if !was_running && !opts.allow_start {
        return Ok(InspectOutcome::NeedsStart {
            distro: name.to_string(),
        });
    }
    let prober = Prober::new(runner, program, name, cancel).map_err(LifecycleError::Invalid)?;
    let mut notes: Vec<String> = Vec::new();
    let mut started = false;

    if !was_running {
        // The explicit, user-approved start: same call as the Start action.
        let wsl = WslClient::new(runner.clone(), program);
        let out = wsl
            .start(name)
            .map_err(|e| map_run("Starting the distribution", e))?;
        if !out.success() {
            return Err(LifecycleError::Failed(format!(
                "WSL could not start '{name}', so it could not be inspected (exit code {}).",
                out.status
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "unknown".into())
            )));
        }
        started = true;
        notes.push("Inspection started this distribution (you allowed it). WSL stops an idle distribution by itself.".into());
    }

    let mut answered = 0u32;
    let mut take = |label: &str,
                    r: Result<Probed, RunError>,
                    notes: &mut Vec<String>|
     -> Result<Option<String>, LifecycleError> {
        match r {
            Ok(Probed::Value(v)) => {
                answered += 1;
                Ok(Some(v))
            }
            Ok(Probed::Absent) => {
                answered += 1;
                notes.push(format!("{label} could not be read."));
                Ok(None)
            }
            Err(RunError::Cancelled) => Err(map_run(label, RunError::Cancelled)),
            Err(e) => {
                notes.push(format!("{label}: {e}."));
                Ok(None)
            }
        }
    };

    let os_text = take(
        "The distribution's os-release",
        prober.os_release(),
        &mut notes,
    )?;
    let kernel = take("The kernel version", prober.kernel(), &mut notes)?;
    let arch = take("The architecture", prober.arch(), &mut notes)?;
    let user = take("The default user", prober.user_name(), &mut notes)?;
    let uid_text = take("The user id", prober.user_id(), &mut notes)?;

    if answered == 0 {
        return Err(LifecycleError::Failed(format!("'{name}' did not answer any inspection command. It may be inaccessible or broken; nothing was changed.")));
    }

    let os_release = os_text.map(|t| parse_os_release(&t));
    let adapter = match &os_release {
        Some(o) => registry.from_os_release(&o.id, &o.id_like).adapter,
        None => registry.from_registered_name(name),
    };
    let support = adapter.packages();

    // Probe the adapter's own manager first; probe the rest only when it is missing or unknown.
    let mut managers: Vec<ManagerProbe> = Vec::new();
    let probe_one = |bin: &str,
                     managers: &mut Vec<ManagerProbe>,
                     notes: &mut Vec<String>|
     -> Result<bool, LifecycleError> {
        match prober.manager_version(bin) {
            Ok(Probed::Value(text)) => {
                managers.push(ManagerProbe {
                    binary: bin.to_string(),
                    present: true,
                    version: parse_version(bin, &text),
                });
                Ok(true)
            }
            Ok(Probed::Absent) => {
                managers.push(ManagerProbe {
                    binary: bin.to_string(),
                    present: false,
                    version: None,
                });
                Ok(false)
            }
            Err(RunError::Cancelled) => {
                Err(map_run("Probing the package manager", RunError::Cancelled))
            }
            Err(e) => {
                notes.push(format!("Could not probe {bin}: {e}."));
                Ok(false)
            }
        }
    };
    let own = support.binary();
    let found_own = match own {
        Some(b) => probe_one(b, &mut managers, &mut notes)?,
        None => false,
    };
    if !found_own {
        for bin in PROBE_BINARIES {
            if Some(bin) == own {
                continue;
            }
            probe_one(bin, &mut managers, &mut notes)?;
        }
    }

    Ok(InspectOutcome::Done {
        inspection: Inspection {
            distro: name.to_string(),
            at: now,
            was_running,
            started_by_inspection: started,
            os_release,
            kernel,
            arch,
            default_user: user,
            uid: uid_text.and_then(|t| t.trim().parse::<u32>().ok()),
            managers,
            notes,
        },
    })
}

// ---------------------------------------------------------------------------------------------
// Persisted cache
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Default, Serialize, Deserialize)]
struct FileData {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    items: Vec<Inspection>,
}

pub struct InspectionStore {
    path: PathBuf,
}

impl InspectionStore {
    pub fn new(dir: &Path) -> Self {
        Self {
            path: dir.join(INSPECTIONS_FILE),
        }
    }

    fn read(&self) -> Result<FileData, String> {
        match fs::read_to_string(&self.path) {
            Ok(t) => serde_json::from_str::<FileData>(&t).map_err(|e| e.to_string()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(FileData::default()),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Unreadable/corrupt file = nothing cached (the file is kept for the user).
    pub fn all(&self) -> Vec<Inspection> {
        self.read().map(|d| d.items).unwrap_or_default()
    }

    pub fn get(&self, distro: &str) -> Option<Inspection> {
        self.all().into_iter().find(|i| i.distro == distro)
    }

    pub fn put(&self, insp: Inspection) -> io::Result<()> {
        let mut data = match self.read() {
            Ok(d) => d,
            Err(_) => {
                let aside = self
                    .path
                    .with_file_name(format!("{INSPECTIONS_FILE}.invalid-{}", insp.at));
                fs::rename(&self.path, aside)?;
                FileData::default()
            }
        };
        data.version = 1;
        data.items.retain(|i| i.distro != insp.distro);
        data.items.push(insp);
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        fs::write(
            &tmp,
            serde_json::to_string_pretty(&data).map_err(io::Error::other)?,
        )?;
        fs::rename(&tmp, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::testutil::TempDir;
    use crate::model::DistroKind::{Internal, Regular};
    use crate::model::DistroState::{Running, Stopped};
    use crate::testing::{fake_report, out_code, out_ok, FakeRunner};

    const OS_ARCH: &[u8] =
        b"NAME=\"Arch Linux\"\nID=arch\nPRETTY_NAME=\"Arch Linux\"\nBUILD_ID=rolling\n";
    const OS_DEBIAN: &[u8] =
        b"PRETTY_NAME=\"Debian GNU/Linux 12 (bookworm)\"\nID=debian\nVERSION_ID=\"12\"\n";
    const OS_ALPINE: &[u8] = b"NAME=\"Alpine Linux\"\nID=alpine\nVERSION_ID=3.20.0\n";

    fn rules(name: &str, os: &[u8], pm: &[(&str, bool)]) -> FakeRunner {
        let mut f = FakeRunner::new()
            .on(
                "wsl.exe",
                &["-d", name, "--exec", "cat", "/etc/os-release"],
                out_ok(os),
            )
            .on(
                "wsl.exe",
                &["-d", name, "--exec", "uname", "-r"],
                out_ok(b"5.15.153.1-microsoft-standard-WSL2\n"),
            )
            .on(
                "wsl.exe",
                &["-d", name, "--exec", "uname", "-m"],
                out_ok(b"x86_64\n"),
            )
            .on(
                "wsl.exe",
                &["-d", name, "--exec", "id", "-un"],
                out_ok(b"dana\n"),
            )
            .on(
                "wsl.exe",
                &["-d", name, "--exec", "id", "-u"],
                out_ok(b"1000\n"),
            );
        for bin in PROBE_BINARIES {
            let present = pm
                .iter()
                .find(|(b, _)| *b == bin)
                .map(|(_, p)| *p)
                .unwrap_or(false);
            let ver: &[u8] = match bin {
                "apt-get" => b"apt 2.6.1 (amd64)\n",
                "pacman" => b"\n .--.   Pacman v7.0.0 - libalpm v15.0.0\n",
                "dnf" => b"4.18.2\n",
                _ => b"x 1\n",
            };
            f = f.on(
                "wsl.exe",
                &["-d", name, "--exec", bin, "--version"],
                if present {
                    out_ok(ver)
                } else {
                    out_code(127, b"")
                },
            );
        }
        f
    }

    fn run(
        f: FakeRunner,
        report: &WslReport,
        name: &str,
        allow: bool,
    ) -> (Result<InspectOutcome, LifecycleError>, Arc<FakeRunner>) {
        let fake = Arc::new(f);
        let r: Arc<dyn CommandRunner> = fake.clone();
        let reg = DistroRegistry::with_builtin();
        let out = inspect(
            &r,
            "wsl.exe",
            report,
            name,
            InspectOptions { allow_start: allow },
            &CancelToken::new(),
            &reg,
            123,
        );
        (out, fake)
    }

    fn done(o: Result<InspectOutcome, LifecycleError>) -> Inspection {
        match o.unwrap() {
            InspectOutcome::Done { inspection } => inspection,
            other => panic!("expected Done, got {other:?}"),
        }
    }

    #[test]
    fn running_arch_is_inspected_read_only() {
        let report = fake_report(&[("archlinux", Running, Regular, 2)], Some("archlinux"));
        let (o, fake) = run(
            rules("archlinux", OS_ARCH, &[("pacman", true)]),
            &report,
            "archlinux",
            false,
        );
        let i = done(o);
        assert!(i.was_running && !i.started_by_inspection);
        assert_eq!(
            i.kernel.as_deref(),
            Some("5.15.153.1-microsoft-standard-WSL2")
        );
        assert_eq!(i.arch.as_deref(), Some("x86_64"));
        assert_eq!(i.default_user.as_deref(), Some("dana"));
        assert_eq!(i.uid, Some(1000));
        assert_eq!(i.os_release.as_ref().unwrap().id, "arch");
        let (caps, _) = i.package_caps(
            DistroRegistry::with_builtin()
                .by_id("arch")
                .unwrap()
                .packages(),
        );
        assert_eq!(caps.manager, PackageManager::Pacman);
        assert!(
            matches!(caps.availability, Availability::Present { version: Some(ref v) } if v == "Pacman v7.0.0")
        );
        // Never started it, never escalated, never ran anything but allow-listed probes.
        let calls = fake.call_lines();
        assert!(!calls
            .iter()
            .any(|c| c.contains("/bin/true") || c.contains("sudo") || c.contains("--terminate")));
        assert!(calls
            .iter()
            .all(|c| c.starts_with("wsl.exe -d archlinux --exec ")));
        // Own manager found: no probing of the others.
        assert_eq!(calls.iter().filter(|c| c.ends_with("--version")).count(), 1);
    }

    #[test]
    fn stopped_distro_needs_permission_and_runs_nothing() {
        let report = fake_report(&[("Debian", Stopped, Regular, 2)], None);
        let (o, fake) = run(FakeRunner::new(), &report, "Debian", false);
        assert_eq!(
            o.unwrap(),
            InspectOutcome::NeedsStart {
                distro: "Debian".into()
            }
        );
        assert!(fake.calls().is_empty());
    }

    #[test]
    fn stopped_distro_with_permission_reports_that_it_started_it() {
        let report = fake_report(&[("Debian", Stopped, Regular, 2)], None);
        let f = rules("Debian", OS_DEBIAN, &[("apt-get", true)]).on(
            "wsl.exe",
            &["-d", "Debian", "--exec", "/bin/true"],
            out_ok(b""),
        );
        let (o, fake) = run(f, &report, "Debian", true);
        let i = done(o);
        assert!(!i.was_running && i.started_by_inspection);
        assert!(i
            .notes
            .iter()
            .any(|n| n.contains("started this distribution")));
        assert_eq!(fake.call_lines()[0], "wsl.exe -d Debian --exec /bin/true");
    }

    #[test]
    fn failed_start_and_unreachable_distros_are_errors_not_guesses() {
        let report = fake_report(&[("Debian", Stopped, Regular, 2)], None);
        let f = FakeRunner::new().on(
            "wsl.exe",
            &["-d", "Debian", "--exec", "/bin/true"],
            out_code(1, b""),
        );
        assert!(matches!(
            run(f, &report, "Debian", true).0,
            Err(LifecycleError::Failed(_))
        ));

        let running = fake_report(&[("Debian", Running, Regular, 2)], None);
        let mut f = FakeRunner::new();
        for tail in [
            &["cat", "/etc/os-release"][..],
            &["uname", "-r"],
            &["uname", "-m"],
            &["id", "-un"],
            &["id", "-u"],
        ] {
            let mut a = vec!["-d", "Debian", "--exec"];
            a.extend_from_slice(tail);
            f = f.on(
                "wsl.exe",
                &a,
                Err(RunError::Timeout(std::time::Duration::from_secs(12))),
            );
        }
        let e = run(f, &running, "Debian", false).0.unwrap_err();
        assert!(matches!(e, LifecycleError::Failed(m) if m.contains("did not answer")));
    }

    #[test]
    fn internal_busy_and_unknown_distros_are_refused() {
        let report = fake_report(
            &[
                ("docker-desktop", Running, Internal, 2),
                ("Debian", Running, Regular, 2),
            ],
            None,
        );
        assert!(matches!(
            run(FakeRunner::new(), &report, "docker-desktop", true).0,
            Err(LifecycleError::Internal(_))
        ));
        assert!(matches!(
            run(FakeRunner::new(), &report, "Nope", true).0,
            Err(LifecycleError::NotFound(_))
        ));
        assert!(matches!(
            run(FakeRunner::new(), &report, "-x", true).0,
            Err(LifecycleError::Invalid(_))
        ));
    }

    #[test]
    fn cancellation_stops_inspection() {
        let report = fake_report(&[("Debian", Running, Regular, 2)], None);
        let f = FakeRunner::new().on_fn(
            "wsl.exe",
            &["-d", "Debian", "--exec", "cat", "/etc/os-release"],
            |_, _, _| Err(RunError::Cancelled),
        );
        let e = run(f, &report, "Debian", false).0.unwrap_err();
        assert!(matches!(e, LifecycleError::Failed(m) if m.contains("cancelled")));
    }

    #[test]
    fn unsupported_manager_is_reported_as_found_but_unsupported() {
        let report = fake_report(&[("Alpine", Running, Regular, 2)], None);
        let (o, _) = run(
            rules("Alpine", OS_ALPINE, &[("apk", true)]),
            &report,
            "Alpine",
            false,
        );
        let i = done(o);
        let adapter = DistroRegistry::with_builtin()
            .from_os_release("alpine", &[])
            .adapter
            .packages();
        let (caps, notes) = i.package_caps(adapter);
        assert_eq!(caps.manager, PackageManager::Unknown);
        assert_eq!(caps.install.support, crate::packages::Support::Unsupported);
        assert!(notes.iter().any(|n| n.contains("apk")));
    }

    #[test]
    fn manager_is_probed_not_assumed_from_the_name() {
        // A distribution whose os-release says Debian but which has no apt-get.
        let report = fake_report(&[("Debian", Running, Regular, 2)], None);
        let (o, fake) = run(rules("Debian", OS_DEBIAN, &[]), &report, "Debian", false);
        let i = done(o);
        let (caps, _) = i.package_caps(
            DistroRegistry::with_builtin()
                .by_id("debian")
                .unwrap()
                .packages(),
        );
        assert_eq!(caps.availability, Availability::Absent);
        assert_eq!(caps.search.support, crate::packages::Support::Unsupported);
        // The others were probed because the expected one was missing.
        assert_eq!(
            fake.call_lines()
                .iter()
                .filter(|c| c.ends_with("--version"))
                .count(),
            PROBE_BINARIES.len()
        );
    }

    #[test]
    fn unidentified_distro_adopts_the_single_probed_supported_manager() {
        let report = fake_report(&[("MyBox", Running, Regular, 2)], None);
        let (o, _) = run(
            rules("MyBox", b"ID=mystery\n", &[("dnf", true)]),
            &report,
            "MyBox",
            false,
        );
        let i = done(o);
        let adapter = DistroRegistry::with_builtin()
            .from_os_release("mystery", &[])
            .adapter
            .packages();
        let (caps, notes) = i.package_caps(adapter);
        assert_eq!(caps.manager, PackageManager::Dnf);
        assert!(notes.iter().any(|n| n.contains("found by probing")));
    }

    #[test]
    fn two_managers_on_an_unidentified_distro_are_not_guessed_between() {
        let managers = vec![
            ManagerProbe {
                binary: "apt-get".into(),
                present: true,
                version: None,
            },
            ManagerProbe {
                binary: "dnf".into(),
                present: true,
                version: None,
            },
        ];
        let (s, a, _) = resolve_packages(PackageSupport::none(), &managers);
        assert_eq!(s.manager, PackageManager::Unknown);
        assert_eq!(a, Availability::Absent);
    }

    #[test]
    fn not_inspected_means_not_checked() {
        let (_, a, _) = resolve_packages(PackageSupport::for_manager(PackageManager::Apt), &[]);
        assert_eq!(a, Availability::NotChecked);
    }

    #[test]
    fn store_roundtrips_replaces_and_survives_corruption() {
        let dir = TempDir::new("inspect-store");
        let store = InspectionStore::new(dir.path());
        assert!(store.all().is_empty());
        let mk = |at| Inspection {
            distro: "Debian".into(),
            at,
            was_running: true,
            started_by_inspection: false,
            os_release: None,
            kernel: Some("k".into()),
            arch: None,
            default_user: None,
            uid: None,
            managers: vec![],
            notes: vec![],
        };
        store.put(mk(1)).unwrap();
        store.put(mk(2)).unwrap();
        assert_eq!(store.all().len(), 1);
        assert_eq!(store.get("Debian").unwrap().at, 2);
        assert!(store.get("Other").is_none());
        fs::write(dir.path().join(INSPECTIONS_FILE), "{ not json").unwrap();
        assert!(store.all().is_empty());
        store.put(mk(3)).unwrap();
        assert_eq!(store.get("Debian").unwrap().at, 3);
        assert!(fs::read_dir(dir.path()).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".invalid-")));
    }
}
