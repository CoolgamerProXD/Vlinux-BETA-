//! End-to-end detection scenarios against a scripted `wsl.exe` (no real WSL or distribution is
//! needed or touched). Every scenario also asserts the read-only guarantee.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use vlinux_runtime::adapters::{DistroRegistry, PackageManager};
use vlinux_runtime::detect::{detect, fetch_catalog, DetectOptions, MAX_OS_RELEASE_QUERIES};
use vlinux_runtime::exec::RunError;
use vlinux_runtime::model::*;
use vlinux_runtime::testing::*;
use vlinux_runtime::wsl::{is_allowed_invocation, Programs, WslClient};

const STATUS: &str = "Default Distribution: Ubuntu-24.04\nDefault Version: 2\n";
const VERSION: &str = "WSL version: 2.4.13.0\nKernel version: 5.15.167.4-1\nWSLg version: 1.0.65\nMSRDC version: 1.2.5716\nWindows version: 10.0.26100.1\n";
const NO_DISTROS: &str = "Windows Subsystem for Linux has no installed distributions.\nDistributions can be installed by visiting the Microsoft Store:\nhttps://aka.ms/wslstore\n";
const NOT_INSTALLED: &str = "The Windows Subsystem for Linux is not installed. You can install by running 'wsl.exe --install'.\nFor more information please visit https://aka.ms/wslinstall\n";
const ARCH_OS_RELEASE: &str = "NAME=\"Arch Linux\"\nPRETTY_NAME=\"Arch Linux\"\nID=arch\nBUILD_ID=rolling\n";

fn host() -> HostEnv {
    HostEnv { is_windows: true, windows_build: Some(26100) }
}

fn reg_keys_absent(f: FakeRunner) -> FakeRunner {
    f.on("reg.exe", &["query", r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending"], out_code(1, b""))
        .on("reg.exe", &["query", r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update\RequiredReboot"], out_code(1, b""))
}

fn reg_pending(f: FakeRunner) -> FakeRunner {
    f.on("reg.exe", &["query", r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending"], out_ok(b"ok"))
}

fn healthy(list: &str) -> FakeRunner {
    FakeRunner::new()
        .on("wsl.exe", &["--status"], utf16_ok(STATUS))
        .on("wsl.exe", &["--version"], utf16_ok(VERSION))
        .on("wsl.exe", &["--list", "--verbose"], utf16_ok(list))
}

fn os_release(f: FakeRunner, name: &str, text: &str) -> FakeRunner {
    f.on("wsl.exe", &["-d", name, "--exec", "cat", "/etc/os-release"], out_ok(text.as_bytes()))
}

fn run_with(fake: &Arc<FakeRunner>, managed: &[&str], identify: bool, h: HostEnv) -> WslReport {
    let wsl = WslClient::new(fake.clone(), "wsl.exe");
    let managed: HashSet<String> = managed.iter().map(|s| s.to_string()).collect();
    detect(&wsl, &Programs::fake(), &DistroRegistry::with_builtin(), &DetectOptions { host: h, managed: &managed, identify_running: identify })
}

fn run(fake: &Arc<FakeRunner>) -> WslReport {
    run_with(fake, &[], true, host())
}

/// Detection must never change anything: only status/version/list queries, `reg query`, and a
/// `cat /etc/os-release` inside an already-running distribution.
fn assert_read_only(fake: &FakeRunner) {
    for c in fake.calls() {
        match c.program.as_str() {
            "wsl.exe" => {
                assert!(is_allowed_invocation(&c.args), "not allow-listed: {}", c.display());
                let a: Vec<&str> = c.args.iter().map(|s| s.as_str()).collect();
                let ok = matches!(a.as_slice(), ["--status"] | ["--version"] | ["--list", "--verbose"] | ["-d", _, "--exec", "cat", "/etc/os-release"]);
                assert!(ok, "detection ran a non-query command: {}", c.display());
            }
            "reg.exe" => assert_eq!(c.args[0], "query", "{}", c.display()),
            other => panic!("unexpected program {other}"),
        }
    }
}

fn distro<'a>(r: &'a WslReport, name: &str) -> &'a Distro {
    r.distros.iter().find(|d| d.name == name).unwrap_or_else(|| panic!("no distro {name}"))
}

// ---- WSL missing / broken ------------------------------------------------------------------

#[test]
fn wsl_exe_missing_is_not_installed() {
    let fake = Arc::new(reg_keys_absent(FakeRunner::new().on("wsl.exe", &["--status"], Err(RunError::NotFound("wsl.exe".into())))));
    let r = run(&fake);
    assert_eq!(r.status, RuntimeStatus::NotInstalled);
    assert_eq!(r.pending_reboot, Some(false));
    assert!(r.distros.is_empty());
    assert_read_only(&fake);
}

#[test]
fn wsl_stub_message_is_not_installed_in_any_language() {
    // The stub prints a localised message; the aka.ms link is what we key on.
    let german = "Das Windows-Subsystem für Linux ist nicht installiert.\nhttps://aka.ms/wslinstall\n";
    for text in [NOT_INSTALLED, german] {
        let fake = Arc::new(reg_keys_absent(FakeRunner::new().on("wsl.exe", &["--status"], utf16_code(1, text))));
        assert_eq!(run(&fake).status, RuntimeStatus::NotInstalled);
        assert_read_only(&fake);
    }
}

#[test]
fn pending_reboot_when_wsl_missing_is_restart_required() {
    let fake = Arc::new(reg_pending(FakeRunner::new().on("wsl.exe", &["--status"], utf16_code(1, NOT_INSTALLED))));
    let r = run(&fake);
    assert_eq!(r.status, RuntimeStatus::RestartRequired);
    assert_eq!(r.pending_reboot, Some(true));
    assert_read_only(&fake);
}

#[test]
fn feature_problem_reports_code_and_hint() {
    let fake = Arc::new(reg_keys_absent(FakeRunner::new().on(
        "wsl.exe",
        &["--status"],
        utf16_code(1, "WSL2 is not supported with your current machine configuration.\nError code: Wsl/Service/CreateInstance/CreateVm/HCS/0x80370102\n"),
    )));
    match run(&fake).status {
        RuntimeStatus::FeatureProblem { code, hint } => {
            assert_eq!(code.as_deref(), Some("0x80370102"));
            assert!(hint.to_lowercase().contains("virtualisation"), "{hint}");
        }
        s => panic!("{s:?}"),
    }
    assert_read_only(&fake);
}

#[test]
fn feature_problem_with_pending_reboot_asks_for_restart() {
    let fake = Arc::new(reg_pending(FakeRunner::new().on("wsl.exe", &["--status"], utf16_code(1, "Error: 0x8007019e\n"))));
    assert_eq!(run(&fake).status, RuntimeStatus::RestartRequired);
}

#[test]
fn unrecognised_failure_is_an_error_not_a_guess() {
    let fake = Arc::new(reg_keys_absent(FakeRunner::new().on("wsl.exe", &["--status"], utf16_code(7, "Something unexpected happened\n"))));
    match run(&fake).status {
        RuntimeStatus::Error { message } => assert!(message.contains("Something unexpected"), "{message}"),
        s => panic!("{s:?}"),
    }
}

#[test]
fn timeouts_are_reported_with_context() {
    let fake = Arc::new(FakeRunner::new().on("wsl.exe", &["--status"], Err(RunError::Timeout(Duration::from_secs(12)))));
    match run(&fake).status {
        RuntimeStatus::Error { message } => assert!(message.contains("did not answer within 12"), "{message}"),
        s => panic!("{s:?}"),
    }
    let fake = Arc::new(
        FakeRunner::new()
            .on("wsl.exe", &["--status"], utf16_ok(STATUS))
            .on("wsl.exe", &["--version"], utf16_ok(VERSION))
            .on("wsl.exe", &["--list", "--verbose"], Err(RunError::Timeout(Duration::from_secs(12)))),
    );
    assert!(matches!(run(&fake).status, RuntimeStatus::Error { .. }));
    assert_read_only(&fake);
}

#[test]
fn malformed_list_output_is_an_error() {
    let fake = Arc::new(healthy("this is\nnot a table at all\n"));
    match run(&fake).status {
        RuntimeStatus::Error { message } => assert!(message.contains("could not understand"), "{message}"),
        s => panic!("{s:?}"),
    }
    let fake = Arc::new(healthy(""));
    assert!(matches!(run(&fake).status, RuntimeStatus::Error { .. }));
}

#[test]
fn partially_malformed_list_keeps_good_rows_and_warns() {
    let list = "  NAME   STATE   VERSION\n* Ubuntu-24.04   Stopped   2\n  Bad/Name   Stopped   2\n  Weird   Stopped   9\n";
    let fake = Arc::new(healthy(list));
    let r = run(&fake);
    assert_eq!(r.status, RuntimeStatus::Ready);
    assert_eq!(r.distros.len(), 1);
    assert!(r.warnings.iter().any(|w| w.contains("Ignored")), "{:?}", r.warnings);
}

#[test]
fn unsupported_host_makes_no_calls() {
    let fake = Arc::new(FakeRunner::new());
    let r = run_with(&fake, &[], true, HostEnv { is_windows: false, windows_build: None });
    assert!(matches!(r.status, RuntimeStatus::UnsupportedHost { .. }));
    assert!(fake.calls().is_empty());

    let r = run_with(&fake, &[], true, HostEnv { is_windows: true, windows_build: Some(17134) });
    assert!(matches!(r.status, RuntimeStatus::UnsupportedHost { .. }));
    assert_eq!(r.wsl2_available, Some(false));
    assert!(fake.calls().is_empty());
}

// ---- encodings ----------------------------------------------------------------------------

#[test]
fn utf16_with_bom_and_plain_utf8_both_decode() {
    let list = "  NAME      STATE           VERSION\n* Ubuntu-24.04    Running         2\n";
    let mut bom = vec![0xFF, 0xFE];
    bom.extend(utf16le(list));
    let utf8 = list.replace('\n', "\r\n").into_bytes();
    // UTF-16LE that has been through a byte-oriented pipe: NULs sprinkled between ASCII.
    let nul_ascii: Vec<u8> = list.bytes().flat_map(|b| [b, 0]).collect();
    for bytes in [utf16le(list), bom, utf8, nul_ascii] {
        let fake = Arc::new(
            FakeRunner::new()
                .on("wsl.exe", &["--status"], utf16_ok(STATUS))
                .on("wsl.exe", &["--version"], utf16_ok(VERSION))
                .on("wsl.exe", &["--list", "--verbose"], out_ok(&bytes)),
        );
        let r = run_with(&fake, &[], false, host());
        assert_eq!(r.status, RuntimeStatus::Ready);
        assert_eq!(r.distros.len(), 1);
        assert_eq!(r.distros[0].name, "Ubuntu-24.04");
        assert_eq!(r.distros[0].state, DistroState::Running);
        assert!(r.distros[0].is_default);
    }
}

// ---- no distributions ---------------------------------------------------------------------

#[test]
fn no_distributions_variants() {
    // (a) --status succeeds, --list prints the "no distributions" message with a failure code.
    let fake = Arc::new(
        FakeRunner::new()
            .on("wsl.exe", &["--status"], utf16_ok("Default Version: 2\n"))
            .on("wsl.exe", &["--version"], utf16_ok(VERSION))
            .on("wsl.exe", &["--list", "--verbose"], utf16_code(1, NO_DISTROS)),
    );
    let r = run(&fake);
    assert_eq!(r.status, RuntimeStatus::NoDistributions);
    assert_eq!(r.default_wsl_version, Some(2));
    assert!(r.distros.is_empty());
    assert_read_only(&fake);

    // (b) --status itself reports the condition.
    let fake = Arc::new(
        FakeRunner::new()
            .on("wsl.exe", &["--status"], utf16_code(1, NO_DISTROS))
            .on("wsl.exe", &["--version"], utf16_ok(VERSION))
            .on("wsl.exe", &["--list", "--verbose"], utf16_ok("")),
    );
    assert_eq!(run(&fake).status, RuntimeStatus::NoDistributions);
}

#[test]
fn only_internal_distributions_means_none_for_the_user() {
    let list = "  NAME              STATE           VERSION\n* docker-desktop    Running         2\n";
    let fake = Arc::new(healthy(list));
    let r = run(&fake);
    assert_eq!(r.status, RuntimeStatus::NoDistributions);
    assert_eq!(r.distros[0].kind, DistroKind::Internal);
    assert_eq!(r.suggested_distro, None);
    // The internal distribution must not be queried even though it is running.
    assert!(!fake.call_lines().iter().any(|l| l.contains("-d")));
}

// ---- distributions ------------------------------------------------------------------------

#[test]
fn single_stopped_distro_is_not_started_to_identify_it() {
    let list = "  NAME            STATE           VERSION\n* Ubuntu-24.04    Stopped         2\n";
    let fake = Arc::new(healthy(list));
    let r = run(&fake);
    assert_eq!(r.status, RuntimeStatus::Ready);
    assert_eq!(r.wsl_version.as_deref(), Some("2.4.13.0"));
    assert_eq!(r.kernel_version.as_deref(), Some("5.15.167.4-1"));
    assert_eq!(r.default_wsl_version, Some(2));
    assert_eq!(r.wsl2_available, Some(true));
    assert_eq!(r.pending_reboot, None, "no restart probe when WSL is healthy");
    let d = distro(&r, "Ubuntu-24.04");
    assert_eq!(d.state, DistroState::Stopped);
    assert_eq!(d.adapter.id, "ubuntu");
    assert_eq!(d.identity_source, IdentitySource::RegisteredName);
    assert_eq!(d.origin, Origin::Existing);
    assert_eq!(r.suggested_distro.as_deref(), Some("Ubuntu-24.04"));
    assert_eq!(fake.call_lines(), vec!["wsl.exe --status", "wsl.exe --version", "wsl.exe --list --verbose"]);
    assert_read_only(&fake);
}

#[test]
fn many_distros_arch_running_default_is_offered_and_identified_without_starting_others() {
    let list = "  NAME              STATE           VERSION\n* archlinux         Running         2\n  Ubuntu-24.04      Stopped         2\n  Debian            Stopped         1\n  docker-desktop    Running         2\n  FedoraLinux-42    Stopped         2\n";
    let fake = Arc::new(os_release(healthy(list), "archlinux", ARCH_OS_RELEASE));
    let r = run(&fake);
    assert_eq!(r.status, RuntimeStatus::Ready);
    assert_eq!(r.distros.len(), 5);
    assert_eq!(r.default_distro.as_deref(), Some("archlinux"));
    assert_eq!(r.suggested_distro.as_deref(), Some("archlinux"));

    let arch = distro(&r, "archlinux");
    assert_eq!(arch.adapter.id, "arch");
    assert_eq!(arch.adapter.package_manager, PackageManager::Pacman);
    assert_eq!(arch.identity_source, IdentitySource::OsRelease);
    assert_eq!(arch.display_label, "Arch Linux");
    assert_eq!(arch.origin, Origin::Existing, "an existing Arch is never reported as installed by VLINUX");

    assert_eq!(distro(&r, "Debian").wsl_version, 1);
    assert_eq!(distro(&r, "Debian").adapter.package_manager, PackageManager::Apt);
    assert_eq!(distro(&r, "FedoraLinux-42").adapter.package_manager, PackageManager::Dnf);
    assert_eq!(distro(&r, "docker-desktop").kind, DistroKind::Internal);

    // Exactly one distribution was queried: the running, non-internal one.
    let queries: Vec<String> = fake.call_lines().into_iter().filter(|l| l.contains(" -d ")).collect();
    assert_eq!(queries, vec!["wsl.exe -d archlinux --exec cat /etc/os-release"]);
    assert_read_only(&fake);
}

#[test]
fn distribution_families_are_recognised_from_registered_names() {
    let list = "  NAME            STATE     VERSION\n* Debian          Stopped   2\n  Ubuntu          Stopped   2\n  FedoraLinux-42  Stopped   2\n  archlinux       Stopped   2\n  kali-linux      Stopped   2\n  Custom-Distro   Stopped   2\n";
    let r = run(&Arc::new(healthy(list)));
    let ids: Vec<(&str, &str)> = r.distros.iter().map(|d| (d.name.as_str(), d.adapter.id.as_str())).collect();
    assert_eq!(ids, vec![("Debian", "debian"), ("Ubuntu", "ubuntu"), ("FedoraLinux-42", "fedora"), ("archlinux", "arch"), ("kali-linux", "generic"), ("Custom-Distro", "generic")]);
    assert_eq!(distro(&r, "kali-linux").adapter.package_manager, PackageManager::Unknown);
    // Arch ranks first among existing installs, but only as a suggestion.
    assert_eq!(r.suggested_distro.as_deref(), Some("archlinux"));
    assert_eq!(r.default_distro.as_deref(), Some("Debian"));
}

#[test]
fn derivative_distribution_is_identified_through_id_like() {
    let list = "  NAME    STATE     VERSION\n* MyBox   Running   2\n";
    let osr = "NAME=\"EndeavourOS\"\nPRETTY_NAME=\"EndeavourOS\"\nID=endeavouros\nID_LIKE=arch\n";
    let fake = Arc::new(os_release(healthy(list), "MyBox", osr));
    let r = run(&fake);
    let d = distro(&r, "MyBox");
    assert_eq!(d.adapter.id, "arch");
    assert_eq!(d.identity_source, IdentitySource::OsRelease);
    assert_eq!(d.display_label, "EndeavourOS");
}

#[test]
fn unreadable_os_release_falls_back_to_the_name_with_a_warning() {
    let list = "  NAME      STATE     VERSION\n* Ubuntu    Running   2\n";
    let fake = Arc::new(healthy(list).on("wsl.exe", &["-d", "Ubuntu", "--exec", "cat", "/etc/os-release"], out_code(1, b"")));
    let r = run(&fake);
    assert_eq!(r.status, RuntimeStatus::Ready);
    assert_eq!(distro(&r, "Ubuntu").adapter.id, "ubuntu");
    assert_eq!(distro(&r, "Ubuntu").identity_source, IdentitySource::RegisteredName);
    assert!(r.warnings.iter().any(|w| w.contains("/etc/os-release")));

    let fake = Arc::new(healthy(list).on("wsl.exe", &["-d", "Ubuntu", "--exec", "cat", "/etc/os-release"], Err(RunError::Timeout(Duration::from_secs(8)))));
    let r = run(&fake);
    assert!(r.warnings.iter().any(|w| w.contains("Could not identify")));
}

#[test]
fn identification_can_be_turned_off() {
    let list = "  NAME      STATE     VERSION\n* Ubuntu    Running   2\n";
    let fake = Arc::new(healthy(list));
    let r = run_with(&fake, &[], false, host());
    assert_eq!(r.status, RuntimeStatus::Ready);
    assert_eq!(fake.calls().len(), 3);
}

#[test]
fn default_distro_marker_is_authoritative_and_independent_of_suggestion() {
    let list = "  NAME            STATE     VERSION\n  archlinux       Running   2\n* Ubuntu-24.04    Stopped   2\n";
    let fake = Arc::new(os_release(healthy(list), "archlinux", ARCH_OS_RELEASE));
    let r = run(&fake);
    assert_eq!(r.default_distro.as_deref(), Some("Ubuntu-24.04"));
    assert_eq!(r.suggested_distro.as_deref(), Some("archlinux"));
    assert!(distro(&r, "Ubuntu-24.04").is_default);
    assert!(!distro(&r, "archlinux").is_default);
}

#[test]
fn wsl1_only_distros_are_never_suggested() {
    let list = "  NAME      STATE     VERSION\n* Debian    Stopped   1\n";
    let r = run(&Arc::new(healthy(list)));
    assert_eq!(r.suggested_distro, None);
    assert_eq!(r.status, RuntimeStatus::Ready);
}

#[test]
fn distros_installed_by_vlinux_are_labelled_case_insensitively() {
    let list = "  NAME            STATE     VERSION\n* archlinux       Stopped   2\n  Ubuntu-24.04    Stopped   2\n";
    let fake = Arc::new(healthy(list));
    let r = run_with(&fake, &["ubuntu-24.04"], true, host());
    assert_eq!(distro(&r, "Ubuntu-24.04").origin, Origin::InstalledByVlinux);
    assert_eq!(distro(&r, "archlinux").origin, Origin::Existing);
}

#[test]
fn os_release_queries_are_bounded() {
    let mut list = String::from("  NAME   STATE   VERSION\n");
    let mut f = FakeRunner::new()
        .on("wsl.exe", &["--status"], utf16_ok(STATUS))
        .on("wsl.exe", &["--version"], utf16_ok(VERSION));
    for i in 1..=(MAX_OS_RELEASE_QUERIES + 4) {
        list.push_str(&format!("  box{i}   Running   2\n"));
        f = f.on("wsl.exe", &["-d", &format!("box{i}"), "--exec", "cat", "/etc/os-release"], out_ok(b"ID=debian\n"));
    }
    f = f.on("wsl.exe", &["--list", "--verbose"], utf16_ok(&list));
    let fake = Arc::new(f);
    let r = run(&fake);
    assert_eq!(r.distros.len(), MAX_OS_RELEASE_QUERIES + 4);
    let queries = fake.call_lines().iter().filter(|l| l.contains(" -d ")).count();
    assert_eq!(queries, MAX_OS_RELEASE_QUERIES);
}

// ---- catalogue ----------------------------------------------------------------------------

const ONLINE: &str = "The following is a list of valid distributions that can be installed.\nInstall using 'wsl.exe --install <Distro>'.\n\nNAME                            FRIENDLY NAME\nUbuntu                          Ubuntu\nDebian                          Debian GNU/Linux\nkali-linux                      Kali Linux Rolling\nFedoraLinux-42                  Fedora Linux 42\narchlinux                       Arch Linux\n";

#[test]
fn catalogue_marks_supported_and_already_installed_and_lists_supported_first() {
    let list = "  NAME        STATE     VERSION\n* archlinux   Running   2\n";
    let fake = Arc::new(healthy(list).on("wsl.exe", &["--list", "--online"], utf16_ok(ONLINE)));
    let r = run_with(&fake, &[], false, host());
    let wsl = WslClient::new(fake.clone(), "wsl.exe");
    let c = fetch_catalog(&wsl, &DistroRegistry::with_builtin(), &r.distros);
    assert_eq!(c.error, None);
    let names: Vec<&str> = c.entries.iter().map(|e| e.wsl_name.as_str()).collect();
    assert_eq!(names, vec!["Ubuntu", "Debian", "FedoraLinux-42", "archlinux", "kali-linux"]);
    let by = |n: &str| c.entries.iter().find(|e| e.wsl_name == n).unwrap();
    assert!(by("archlinux").already_installed);
    assert!(by("archlinux").supported);
    assert!(!by("Ubuntu").already_installed);
    assert!(!by("kali-linux").supported, "no adapter => offered as unsupported, never installable");
    assert_eq!(by("kali-linux").adapter, None);
    assert_eq!(by("FedoraLinux-42").friendly_name, "Fedora Linux 42");
}

#[test]
fn catalogue_failures_are_reported_not_hidden() {
    let reg = DistroRegistry::with_builtin();
    let fake = Arc::new(FakeRunner::new().on("wsl.exe", &["--list", "--online"], Err(RunError::Timeout(Duration::from_secs(25)))));
    let c = fetch_catalog(&WslClient::new(fake, "wsl.exe"), &reg, &[]);
    assert!(c.entries.is_empty());
    assert!(c.error.unwrap().contains("internet"));

    let fake = Arc::new(FakeRunner::new().on("wsl.exe", &["--list", "--online"], utf16_code(1, "Failed to fetch the list: WININET_E_NAME_NOT_RESOLVED\n")));
    let c = fetch_catalog(&WslClient::new(fake, "wsl.exe"), &reg, &[]);
    assert!(c.error.unwrap().contains("WININET_E_NAME_NOT_RESOLVED"));

    let fake = Arc::new(FakeRunner::new().on("wsl.exe", &["--list", "--online"], utf16_ok("")));
    assert!(fetch_catalog(&WslClient::new(fake, "wsl.exe"), &reg, &[]).error.is_some());
}

#[test]
fn a_new_adapter_is_all_it_takes_to_support_another_distribution() {
    use vlinux_runtime::adapters::DistroAdapter;
    struct Alpine;
    impl DistroAdapter for Alpine {
        fn id(&self) -> &'static str { "alpine" }
        fn display_name(&self) -> &'static str { "Alpine Linux" }
        fn os_release_ids(&self) -> &'static [&'static str] { &["alpine"] }
        fn registered_name_prefixes(&self) -> &'static [&'static str] { &["alpine"] }
        fn wsl_online_prefixes(&self) -> &'static [&'static str] { &["alpine"] }
        fn package_manager(&self) -> PackageManager { PackageManager::Unknown }
    }
    let mut reg = DistroRegistry::with_builtin();
    assert_eq!(reg.from_registered_name("Alpine").id(), "generic");
    reg.register(Box::new(Alpine));
    assert_eq!(reg.from_registered_name("Alpine").id(), "alpine");
    assert_eq!(reg.from_os_release("alpine", &[]).adapter.id(), "alpine");
}
