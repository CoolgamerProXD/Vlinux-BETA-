//! Install-workflow scenarios with a scripted `wsl.exe`. **No distribution is installed**: the
//! scripted installer only returns canned output. The tests prove the safety rules: nothing runs
//! without explicit approval, plans are single-use and expire, existing distributions are never
//! replaced, and no destructive `wsl.exe` verb is ever issued.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use vlinux_runtime::adapters::DistroRegistry;
use vlinux_runtime::detect::{detect, fetch_catalog, DetectOptions};
use vlinux_runtime::exec::{CmdOutput, RunError};
use vlinux_runtime::install::*;
use vlinux_runtime::model::*;
use vlinux_runtime::testing::*;
use vlinux_runtime::wsl::{Programs, WslClient};

const VERSION: &str = "WSL version: 2.4.13.0\nKernel version: 5.15.167.4-1\n";
const ONLINE: &str = "NAME                FRIENDLY NAME\nUbuntu              Ubuntu\nDebian              Debian GNU/Linux\nkali-linux          Kali Linux Rolling\nFedoraLinux-42      Fedora Linux 42\narchlinux           Arch Linux\n";
const LIST_WITH_ARCH: &str = "  NAME        STATE     VERSION\n* archlinux   Running   2\n";
const NO_DISTROS: &str =
    "Windows Subsystem for Linux has no installed distributions.\nhttps://aka.ms/wslstore\n";

const REG_A: &str =
    r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending";
const REG_B: &str =
    r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update\RequiredReboot";

fn host() -> HostEnv {
    HostEnv {
        is_windows: true,
        windows_build: Some(26100),
    }
}

fn base(list: Result<CmdOutput, RunError>) -> FakeRunner {
    FakeRunner::new()
        .on("wsl.exe", &["--status"], utf16_ok("Default Version: 2\n"))
        .on("wsl.exe", &["--version"], utf16_ok(VERSION))
        .on("wsl.exe", &["--list", "--verbose"], list)
        .on("wsl.exe", &["--list", "--online"], utf16_ok(ONLINE))
        .on("reg.exe", &["query", REG_A], out_code(1, b""))
        .on("reg.exe", &["query", REG_B], out_code(1, b""))
}

struct World {
    wsl: WslClient,
    report: WslReport,
    catalog: Catalog,
    fake: Arc<FakeRunner>,
}

fn world_with(fake: FakeRunner) -> World {
    let fake = Arc::new(fake);
    let wsl = WslClient::new(fake.clone(), "wsl.exe");
    let managed = HashSet::new();
    let reg = DistroRegistry::with_builtin();
    let report = detect(
        &wsl,
        &Programs::fake(),
        &reg,
        &DetectOptions {
            host: host(),
            managed: &managed,
            identify_running: false,
        },
    );
    let catalog = fetch_catalog(&wsl, &reg, &report.distros);
    World {
        wsl,
        report,
        catalog,
        fake,
    }
}

fn world_with_arch() -> World {
    world_with(base(utf16_ok(LIST_WITH_ARCH)))
}

fn plan(w: &World, name: &str) -> InstallPlan {
    prepare_distribution_plan(name, &w.report, &w.catalog)
}

const DESTRUCTIVE: [&str; 9] = [
    "--unregister",
    "--import",
    "--export",
    "--set-default",
    "--set-version",
    "--shutdown",
    "--mount",
    "--terminate",
    "--manage",
];

fn assert_no_destructive_calls(fake: &FakeRunner) {
    for c in fake.calls() {
        for a in &c.args {
            assert!(
                !DESTRUCTIVE.contains(&a.as_str()),
                "destructive verb issued: {}",
                c.display()
            );
        }
    }
}

fn install_calls(fake: &FakeRunner) -> Vec<String> {
    fake.call_lines()
        .into_iter()
        .filter(|l| l.contains("--install"))
        .collect()
}

fn install_output(code: i32, text: &str) -> Result<CmdOutput, RunError> {
    utf16_code(code, text)
}

// ---- planning -------------------------------------------------------------------------------

#[test]
fn plan_for_a_new_distribution_lists_what_will_happen_and_what_stays_untouched() {
    let w = world_with_arch();
    let p = plan(&w, "Debian");
    assert!(p.blockers.is_empty(), "{:?}", p.blockers);
    assert_eq!(p.kind, PlanKind::Distribution);
    assert_eq!(p.command_preview, "wsl.exe --install Debian --no-launch");
    assert_eq!(p.requires_admin, AdminRequirement::Never);
    assert!(!p.restart_possible);
    assert!(p.needs_internet);
    assert_eq!(p.existing_untouched, vec!["archlinux"]);
    assert_eq!(p.adapter.as_ref().map(|a| a.id.as_str()), Some("debian"));
    assert!(p.steps.iter().any(|s| s.contains("side by side")));
    assert!(
        w.fake.call_lines().iter().all(|l| !l.contains("--install")),
        "planning must not install anything"
    );
}

#[test]
fn an_existing_distribution_can_never_be_installed_over() {
    let w = world_with_arch();
    let p = plan(&w, "archlinux");
    assert!(
        p.blockers.iter().any(|b| b.contains("already installed")),
        "{:?}",
        p.blockers
    );
    // Case-insensitive: WSL names are.
    assert!(!plan(&w, "ArchLinux").blockers.is_empty());
    let store = PlanStore::default();
    let p = store.insert(p);
    assert!(matches!(
        store.take_approved(&p.plan_id, true, &w.report),
        Err(InstallError::Blocked(_))
    ));
}

#[test]
fn unsupported_unknown_and_unverifiable_distributions_are_blocked() {
    let w = world_with_arch();
    assert!(plan(&w, "kali-linux")
        .blockers
        .iter()
        .any(|b| b.contains("no adapter")));
    assert!(plan(&w, "NotADistro")
        .blockers
        .iter()
        .any(|b| b.contains("official list")));

    // Offline: the catalogue could not be fetched, so nothing can be verified.
    let offline = world_with(
        FakeRunner::new()
            .on("wsl.exe", &["--status"], utf16_ok("Default Version: 2\n"))
            .on("wsl.exe", &["--version"], utf16_ok(VERSION))
            .on(
                "wsl.exe",
                &["--list", "--verbose"],
                utf16_ok(LIST_WITH_ARCH),
            )
            .on(
                "wsl.exe",
                &["--list", "--online"],
                Err(RunError::Timeout(Duration::from_secs(25))),
            ),
    );
    assert!(offline.catalog.error.is_some());
    assert!(plan(&offline, "Debian")
        .blockers
        .iter()
        .any(|b| b.contains("unavailable")));
}

#[test]
fn distribution_plans_are_blocked_until_wsl_itself_works() {
    let missing = world_with(
        FakeRunner::new()
            .on(
                "wsl.exe",
                &["--status"],
                Err(RunError::NotFound("wsl.exe".into())),
            )
            .on("wsl.exe", &["--list", "--online"], utf16_ok(ONLINE))
            .on("reg.exe", &["query", REG_A], out_code(1, b""))
            .on("reg.exe", &["query", REG_B], out_code(1, b"")),
    );
    assert_eq!(missing.report.status, RuntimeStatus::NotInstalled);
    let p = plan(&missing, "Debian");
    assert!(
        p.blockers.iter().any(|b| b.contains("Set up WSL")),
        "{:?}",
        p.blockers
    );
}

#[test]
fn no_distributions_installed_is_a_valid_state_to_install_into() {
    let w = world_with(base(utf16_code(1, NO_DISTROS)));
    assert_eq!(w.report.status, RuntimeStatus::NoDistributions);
    let p = plan(&w, "Ubuntu");
    assert!(p.blockers.is_empty(), "{:?}", p.blockers);
    assert!(p.existing_untouched.is_empty());
}

#[test]
fn wsl_platform_plan_explains_admin_and_restart_and_never_includes_a_distribution() {
    let missing = world_with(
        FakeRunner::new()
            .on(
                "wsl.exe",
                &["--status"],
                Err(RunError::NotFound("wsl.exe".into())),
            )
            .on("reg.exe", &["query", REG_A], out_code(1, b""))
            .on("reg.exe", &["query", REG_B], out_code(1, b"")),
    );
    let p = prepare_wsl_platform_plan(&missing.report);
    assert!(p.blockers.is_empty());
    assert_eq!(p.kind, PlanKind::WslPlatform);
    assert_eq!(p.command_preview, "wsl.exe --install --no-distribution");
    assert_eq!(p.requires_admin, AdminRequirement::MayPrompt);
    assert!(p.restart_possible);
    assert!(p.steps.iter().any(|s| s.contains("UAC")));
    assert!(p.steps.iter().any(|s| s.contains("No Linux distribution")));

    // Already working: no reinstall offered.
    let ok = world_with_arch();
    assert!(!prepare_wsl_platform_plan(&ok.report).blockers.is_empty());
}

// ---- consent, expiry, single use ------------------------------------------------------------

#[test]
fn nothing_starts_without_explicit_approval_and_plans_are_single_use() {
    let w = world_with_arch();
    let store = PlanStore::default();
    let p = store.insert(plan(&w, "Debian"));
    assert!(p.plan_id.starts_with("plan-"));

    assert_eq!(
        store.take_approved(&p.plan_id, false, &w.report),
        Err(InstallError::NotApproved)
    );
    // A refusal does not burn the plan; approving afterwards works...
    let approved = store
        .take_approved(&p.plan_id, true, &w.report)
        .expect("approved");
    assert_eq!(approved.wsl_name.as_deref(), Some("Debian"));
    // ...exactly once.
    assert_eq!(
        store.take_approved(&p.plan_id, true, &w.report),
        Err(InstallError::UnknownPlan)
    );
    assert_eq!(
        store.take_approved("plan-forged", true, &w.report),
        Err(InstallError::UnknownPlan)
    );
    assert!(install_calls(&w.fake).is_empty());
}

#[test]
fn plan_ids_are_unique() {
    let w = world_with_arch();
    let store = PlanStore::default();
    let a = store.insert(plan(&w, "Debian"));
    let b = store.insert(plan(&w, "Debian"));
    assert_ne!(a.plan_id, b.plan_id);
}

#[test]
fn plans_expire() {
    let w = world_with_arch();
    let store = PlanStore::with_ttl(Duration::from_millis(30));
    let p = store.insert(plan(&w, "Debian"));
    std::thread::sleep(Duration::from_millis(80));
    assert_eq!(
        store.take_approved(&p.plan_id, true, &w.report),
        Err(InstallError::PlanExpired)
    );
    assert_eq!(
        store.take_approved(&p.plan_id, true, &w.report),
        Err(InstallError::UnknownPlan)
    );
}

#[test]
fn approval_revalidates_against_the_current_state() {
    let w = world_with_arch();
    let store = PlanStore::default();

    // Someone installed Debian between review and approval.
    let p = store.insert(plan(&w, "Debian"));
    let with_debian = world_with(base(utf16_ok(
        "  NAME   STATE   VERSION\n* archlinux   Running   2\n  Debian   Stopped   2\n",
    )));
    assert_eq!(
        store.take_approved(&p.plan_id, true, &with_debian.report),
        Err(InstallError::AlreadyExists("Debian".into()))
    );

    // WSL broke between review and approval.
    let p = store.insert(plan(&w, "Debian"));
    let broken = world_with(
        FakeRunner::new()
            .on(
                "wsl.exe",
                &["--status"],
                Err(RunError::NotFound("wsl.exe".into())),
            )
            .on("reg.exe", &["query", REG_A], out_code(1, b""))
            .on("reg.exe", &["query", REG_B], out_code(1, b"")),
    );
    assert!(matches!(
        store.take_approved(&p.plan_id, true, &broken.report),
        Err(InstallError::NotReady(_))
    ));

    // A platform plan is only valid while WSL is actually missing.
    let p = store.insert(prepare_wsl_platform_plan(&broken.report));
    assert!(matches!(
        store.take_approved(&p.plan_id, true, &w.report),
        Err(InstallError::NotReady(_))
    ));
}

// ---- running an approved plan ---------------------------------------------------------------

fn run(
    w: &World,
    p: &InstallPlan,
    cancel: &vlinux_runtime::exec::CancelToken,
) -> (InstallOutcome, Vec<InstallEvent>) {
    let mut events = Vec::new();
    let outcome = run_install(&w.wsl, &Programs::fake(), p, "job-1", cancel, &mut |e| {
        events.push(e)
    });
    (outcome, events)
}

#[test]
fn successful_install_streams_progress_and_issues_exactly_one_install_command() {
    let fake = base(utf16_ok(LIST_WITH_ARCH)).on_fn("wsl.exe", &["--install", "Debian", "--no-launch"], |_, _, out| {
        out(&utf16le("Downloading: Debian\n"));
        out(&utf16le("Installing: Debian [=====     ] 45%\r"));
        out(&utf16le("Installing: Debian [==========] 100%\r\n"));
        out_code(0, &utf16le("Downloading: Debian\nInstalling: Debian [==========] 100%\nThe operation completed successfully.\n"))
    });
    let w = world_with(fake);
    let p = plan(&w, "Debian");
    let (outcome, events) = run(&w, &p, &vlinux_runtime::exec::CancelToken::new());
    assert_eq!(outcome, InstallOutcome::Succeeded);
    assert!(events
        .iter()
        .all(|e| e.job_id == "job-1" && e.kind == "progress"));
    assert!(events[0]
        .message
        .contains("wsl.exe --install Debian --no-launch"));
    assert!(events.iter().any(|e| e.percent == Some(45.0)), "{events:?}");
    assert_eq!(events.last().unwrap().percent, Some(100.0));
    assert_eq!(
        install_calls(&w.fake),
        vec!["wsl.exe --install Debian --no-launch"]
    );
    assert_no_destructive_calls(&w.fake);
}

#[test]
fn failed_install_reports_exit_code_message_and_hint_and_touches_nothing_else() {
    let fake = base(utf16_ok(LIST_WITH_ARCH)).on(
        "wsl.exe",
        &["--install", "Debian", "--no-launch"],
        install_output(1, "Downloading: Debian\nFailed to download: WININET_E_CANNOT_CONNECT\nError code: Wsl/InstallDistro/WININET_E_CANNOT_CONNECT\n"),
    );
    let w = world_with(fake);
    let (outcome, _) = run(
        &w,
        &plan(&w, "Debian"),
        &vlinux_runtime::exec::CancelToken::new(),
    );
    match outcome {
        InstallOutcome::Failed {
            exit_code,
            message,
            hint,
        } => {
            assert_eq!(exit_code, Some(1));
            assert!(message.contains("WININET_E_CANNOT_CONNECT"), "{message}");
            assert!(
                hint.contains("did not change any existing distribution"),
                "{hint}"
            );
        }
        o => panic!("{o:?}"),
    }
    assert_no_destructive_calls(&w.fake);
}

#[test]
fn recognised_error_codes_give_actionable_hints() {
    let fake = base(utf16_ok(LIST_WITH_ARCH)).on(
        "wsl.exe",
        &["--install", "Debian", "--no-launch"],
        install_output(1, "Error: 0x80370102\n"),
    );
    let w = world_with(fake);
    match run(
        &w,
        &plan(&w, "Debian"),
        &vlinux_runtime::exec::CancelToken::new(),
    )
    .0
    {
        InstallOutcome::Failed { hint, .. } => {
            assert!(hint.to_lowercase().contains("virtualisation"), "{hint}")
        }
        o => panic!("{o:?}"),
    }
}

#[test]
fn cancellation_stops_the_job_and_reports_cancelled() {
    let fake = base(utf16_ok(LIST_WITH_ARCH)).on_fn(
        "wsl.exe",
        &["--install", "Debian", "--no-launch"],
        |_, cancel, out| {
            out(&utf16le("Downloading: Debian [==        ] 20%\r"));
            // The user pressed Cancel while the child was running; the runner kills it and reports so.
            cancel.cancel();
            assert!(cancel.is_cancelled());
            Err(RunError::Cancelled)
        },
    );
    let w = world_with(fake);
    let token = vlinux_runtime::exec::CancelToken::new();
    let (outcome, events) = run(&w, &plan(&w, "Debian"), &token);
    assert_eq!(outcome, InstallOutcome::Cancelled);
    assert!(events.iter().any(|e| e.percent == Some(20.0)));
    assert!(token.is_cancelled());
    assert_no_destructive_calls(&w.fake);
}

#[test]
fn a_half_registered_distribution_after_failure_is_reported_never_removed() {
    // After the failed install a re-detection shows a registered Debian.
    let state = Arc::new(Mutex::new(false));
    let s2 = state.clone();
    let s3 = state.clone();
    let fake = FakeRunner::new()
        .on("wsl.exe", &["--status"], utf16_ok("Default Version: 2\n"))
        .on("wsl.exe", &["--version"], utf16_ok(VERSION))
        .on("wsl.exe", &["--list", "--online"], utf16_ok(ONLINE))
        .on_fn("wsl.exe", &["--list", "--verbose"], move |_, _, _| {
            if *s2.lock().unwrap() {
                utf16_ok(
                    "  NAME   STATE   VERSION\n* archlinux   Running   2\n  Debian   Stopped   2\n",
                )
            } else {
                utf16_ok(LIST_WITH_ARCH)
            }
        })
        .on_fn(
            "wsl.exe",
            &["--install", "Debian", "--no-launch"],
            move |_, _, _| {
                *s3.lock().unwrap() = true;
                install_output(1, "Error code: Wsl/InstallDistro/ERROR_FILE_CORRUPT\n")
            },
        );
    let w = world_with(fake);
    let (outcome, _) = run(
        &w,
        &plan(&w, "Debian"),
        &vlinux_runtime::exec::CancelToken::new(),
    );
    assert!(matches!(outcome, InstallOutcome::Failed { .. }));

    let managed = HashSet::new();
    let after = detect(
        &w.wsl,
        &Programs::fake(),
        &DistroRegistry::with_builtin(),
        &DetectOptions {
            host: host(),
            managed: &managed,
            identify_running: false,
        },
    );
    let note = leftover_note("Debian", &after).expect("leftover is reported");
    assert!(note.contains("has not removed it"), "{note}");
    assert!(leftover_note("Ubuntu", &after).is_none());
    assert_no_destructive_calls(&w.fake);
    // The pre-existing distribution is still listed and was never addressed by any command.
    assert!(after.distros.iter().any(|d| d.name == "archlinux"));
    assert!(!w.fake.call_lines().iter().any(|l| l.contains("archlinux")));
}

#[test]
fn restart_required_is_explained_not_treated_as_failure() {
    // Exit code 3010 = ERROR_SUCCESS_REBOOT_REQUIRED.
    let fake = base(utf16_ok(LIST_WITH_ARCH)).on("wsl.exe", &["--install", "Debian", "--no-launch"], install_output(3010, "The requested operation is successful. Changes will not be effective until the system is rebooted.\n"));
    let w = world_with(fake);
    match run(
        &w,
        &plan(&w, "Debian"),
        &vlinux_runtime::exec::CancelToken::new(),
    )
    .0
    {
        InstallOutcome::RestartRequired { message } => assert!(message.contains("restart")),
        o => panic!("{o:?}"),
    }
    // Failure text that carries the reboot HRESULT.
    let fake = base(utf16_ok(LIST_WITH_ARCH)).on("wsl.exe", &["--install", "Debian", "--no-launch"], install_output(1, "Error: 0x80070bc2 The requested operation is successful. Changes will not be effective until the system is rebooted.\n"));
    let w = world_with(fake);
    assert!(matches!(
        run(
            &w,
            &plan(&w, "Debian"),
            &vlinux_runtime::exec::CancelToken::new()
        )
        .0,
        InstallOutcome::RestartRequired { .. }
    ));
}

#[test]
fn wsl_platform_install_checks_for_a_pending_restart_afterwards() {
    let build = |pending: bool| {
        let mut f = FakeRunner::new()
            .on("wsl.exe", &["--status"], Err(RunError::NotFound("wsl.exe".into())))
            .on("reg.exe", &["query", REG_B], out_code(1, b""))
            .on("wsl.exe", &["--install", "--no-distribution"], install_output(0, "Installing: Windows Subsystem for Linux\nThe requested operation is successful.\n"));
        f = if pending {
            f.on("reg.exe", &["query", REG_A], out_ok(b"found"))
        } else {
            f.on("reg.exe", &["query", REG_A], out_code(1, b""))
        };
        world_with(f)
    };
    let w = build(false);
    let p = prepare_wsl_platform_plan(&w.report);
    let (outcome, _) = run(&w, &p, &vlinux_runtime::exec::CancelToken::new());
    assert_eq!(outcome, InstallOutcome::Succeeded);
    assert_eq!(
        install_calls(&w.fake),
        vec!["wsl.exe --install --no-distribution"]
    );

    let w = build(true);
    // With the restart-pending key present, detection itself reports RestartRequired, so the
    // platform plan is blocked: VLINUX will not start another installation on top of a pending one.
    assert_eq!(w.report.status, RuntimeStatus::RestartRequired);
    assert!(!prepare_wsl_platform_plan(&w.report).blockers.is_empty());
}

#[test]
fn timeouts_and_missing_wsl_are_reported() {
    let fake = base(utf16_ok(LIST_WITH_ARCH)).on(
        "wsl.exe",
        &["--install", "Debian", "--no-launch"],
        Err(RunError::Timeout(Duration::from_secs(2700))),
    );
    let w = world_with(fake);
    assert_eq!(
        run(
            &w,
            &plan(&w, "Debian"),
            &vlinux_runtime::exec::CancelToken::new()
        )
        .0,
        InstallOutcome::TimedOut
    );

    let fake = base(utf16_ok(LIST_WITH_ARCH)).on(
        "wsl.exe",
        &["--install", "Debian", "--no-launch"],
        Err(RunError::NotFound("wsl.exe".into())),
    );
    let w = world_with(fake);
    assert!(matches!(
        run(
            &w,
            &plan(&w, "Debian"),
            &vlinux_runtime::exec::CancelToken::new()
        )
        .0,
        InstallOutcome::Failed {
            exit_code: None,
            ..
        }
    ));
}

#[test]
fn a_tampered_plan_cannot_smuggle_arguments() {
    let w = world_with_arch();
    let mut p = plan(&w, "Debian");
    p.wsl_name = Some("Debian --unregister archlinux".into());
    let (outcome, _) = run(&w, &p, &vlinux_runtime::exec::CancelToken::new());
    assert!(matches!(outcome, InstallOutcome::Failed { .. }));
    assert!(
        install_calls(&w.fake).is_empty(),
        "the unsafe name never reached the process runner"
    );
    assert_no_destructive_calls(&w.fake);
}

#[test]
fn outcomes_serialise_with_a_stable_tag() {
    // The UI switches on `kind`; it must not change by accident.
    let j = |o: &InstallOutcome| serde_json::to_value(o).unwrap();
    assert_eq!(j(&InstallOutcome::Succeeded)["kind"], "succeeded");
    assert_eq!(j(&InstallOutcome::Cancelled)["kind"], "cancelled");
    assert_eq!(j(&InstallOutcome::TimedOut)["kind"], "timed_out");
    assert_eq!(
        j(&InstallOutcome::RestartRequired {
            message: "m".into()
        })["kind"],
        "restart_required"
    );
    assert_eq!(
        j(&InstallOutcome::Failed {
            exit_code: Some(1),
            message: "m".into(),
            hint: "h".into()
        })["exit_code"],
        1
    );
}
