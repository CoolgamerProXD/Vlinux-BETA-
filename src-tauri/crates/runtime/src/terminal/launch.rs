//! Choosing *which* distribution a terminal opens, and the exact `wsl.exe` command lines used.
//!
//! The frontend only ever names a distribution. That name is validated here against the current
//! WSL report, and the argument lists below are built by VLINUX alone (no shell, no
//! interpolation, never `--user root`, never `--set-default`).

use std::path::PathBuf;

use super::pty::{PtySize, PtySpawn};
use super::TerminalError;
use crate::model::{Distro, DistroKind, DistroState, RuntimeStatus, WslReport};
use crate::parse::valid_distro_name;
use crate::wsl::Programs;

/// A distribution a terminal may open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// Registered WSL name, exactly as WSL lists it.
    pub name: String,
    /// Human label, e.g. "Arch Linux".
    pub label: String,
}

/// One way of starting a shell. Attempts are tried in order only when the previous one failed
/// immediately (see `session`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchAttempt {
    /// Short name shown in the UI, e.g. `default shell`.
    pub label: String,
    pub spec: PtySpawn,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    pub attempts: Vec<LaunchAttempt>,
}

impl LaunchPlan {
    /// A single-attempt plan for an arbitrary program (used by tests and generic backends).
    pub fn single(label: &str, program: &str, args: &[&str]) -> Self {
        LaunchPlan {
            attempts: vec![LaunchAttempt {
                label: label.to_string(),
                spec: PtySpawn {
                    program: program.to_string(),
                    args: args.iter().map(|s| s.to_string()).collect(),
                    env: Vec::new(),
                    cwd: None,
                    size: PtySize::DEFAULT,
                },
            }],
        }
    }
}

fn describe_state(state: DistroState) -> Option<&'static str> {
    match state {
        DistroState::Installing => Some("being installed"),
        DistroState::Uninstalling => Some("being uninstalled"),
        DistroState::Converting => Some("being converted"),
        DistroState::Running | DistroState::Stopped | DistroState::Unknown => None,
    }
}

/// Decide which distribution to open: the explicitly requested one, else the user's selection.
/// Never falls back to "whatever is default" or "the first one" — and never installs anything.
pub fn resolve_target(report: &WslReport, requested: Option<&str>, selected: Option<&str>) -> Result<Target, TerminalError> {
    match &report.status {
        RuntimeStatus::Ready => {}
        RuntimeStatus::NoDistributions => return Err(TerminalError::NoDistributions),
        RuntimeStatus::NotInstalled => {
            return Err(TerminalError::WslUnavailable { reason: "WSL is not installed on this PC.".into() })
        }
        RuntimeStatus::RestartRequired => {
            return Err(TerminalError::WslUnavailable { reason: "Windows needs a restart before WSL can be used.".into() })
        }
        RuntimeStatus::UnsupportedHost { reason } => {
            return Err(TerminalError::WslUnavailable { reason: format!("{reason}") })
        }
        RuntimeStatus::FeatureProblem { hint, .. } => return Err(TerminalError::WslUnavailable { reason: hint.clone() }),
        RuntimeStatus::Error { message } => {
            return Err(TerminalError::WslUnavailable { reason: format!("WSL did not respond ({message}).") })
        }
    }

    let name = requested.or(selected).ok_or(TerminalError::NoSelection)?;
    if !valid_distro_name(name) {
        return Err(TerminalError::InvalidDistro { name: name.to_string() });
    }

    let found: Option<&Distro> = report
        .distros
        .iter()
        .find(|d| d.name == name)
        .or_else(|| report.distros.iter().find(|d| d.name.eq_ignore_ascii_case(name)));

    let Some(d) = found else {
        let available = report.distros.iter().filter(|d| d.kind == DistroKind::Regular).map(|d| d.name.clone()).collect();
        return Err(TerminalError::DistroNotFound { name: name.to_string(), available });
    };
    if d.kind != DistroKind::Regular {
        return Err(TerminalError::DistroNotAllowed { name: d.name.clone() });
    }
    if let Some(state) = describe_state(d.state) {
        return Err(TerminalError::DistroBusy { name: d.name.clone(), state: state.to_string() });
    }
    if !valid_distro_name(&d.name) {
        // WSL reported a name we would not pass back to it.
        return Err(TerminalError::InvalidDistro { name: d.name.clone() });
    }
    Ok(Target { name: d.name.clone(), label: d.display_label.clone() })
}

/// The complete set of argument lists a terminal session may run. Used as a guard at launch time
/// and by tests.
pub fn is_allowed_session_args(args: &[String]) -> bool {
    let a: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    match a.as_slice() {
        ["-d", name, "--cd", "~"] => valid_distro_name(name),
        ["-d", name, "--cd", "~", "--exec", "/bin/sh", "-l"] => valid_distro_name(name),
        _ => false,
    }
}

fn merged_wslenv(existing: Option<String>) -> String {
    let mut items: Vec<String> = existing
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    for var in ["TERM", "COLORTERM"] {
        if !items.iter().any(|i| i.split('/').next() == Some(var)) {
            items.push(var.to_string());
        }
    }
    items.join(":")
}

/// Build the launch plan for `target`.
///
/// 1. `wsl.exe -d <name> --cd ~` — WSL starts the user's *own* login shell as configured in the
///    distribution (`/etc/passwd`): bash, zsh, fish, sh, ... nothing is hardcoded here.
/// 2. Only if that fails immediately (for example the configured shell is missing or broken):
///    `wsl.exe -d <name> --cd ~ --exec /bin/sh -l`, the POSIX shell every distribution has.
///
/// The session runs as the distribution's default user. VLINUX never passes `--user`, so it never
/// starts a root shell and never elevates.
pub fn build_plan(programs: &Programs, target: &Target) -> Result<LaunchPlan, TerminalError> {
    let env = vec![
        ("TERM".to_string(), "xterm-256color".to_string()),
        ("COLORTERM".to_string(), "truecolor".to_string()),
        // Forward TERM/COLORTERM into the distribution (documented WSLENV mechanism).
        ("WSLENV".to_string(), merged_wslenv(std::env::var("WSLENV").ok())),
    ];
    let make = |label: &str, extra: &[&str]| {
        let mut args: Vec<String> = vec!["-d".into(), target.name.clone(), "--cd".into(), "~".into()];
        args.extend(extra.iter().map(|s| s.to_string()));
        LaunchAttempt {
            label: label.to_string(),
            spec: PtySpawn {
                program: programs.wsl.clone(),
                args,
                env: env.clone(),
                cwd: None::<PathBuf>,
                size: PtySize::DEFAULT,
            },
        }
    };
    let plan = LaunchPlan {
        attempts: vec![make("default shell", &[]), make("/bin/sh", &["--exec", "/bin/sh", "-l"])],
    };
    for a in &plan.attempts {
        if !is_allowed_session_args(&a.spec.args) {
            return Err(TerminalError::StartFailed { detail: "internal error: refused a non-allow-listed wsl.exe invocation".into() });
        }
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::testing::{distro, report};

    fn ready(distros: Vec<Distro>) -> WslReport {
        report(RuntimeStatus::Ready, distros)
    }

    #[test]
    fn selected_distribution_is_used() {
        let r = ready(vec![
            distro("Debian", DistroState::Stopped, DistroKind::Regular),
            distro("archlinux", DistroState::Running, DistroKind::Regular),
        ]);
        let t = resolve_target(&r, None, Some("archlinux")).unwrap();
        assert_eq!(t.name, "archlinux");
        assert_eq!(t.label, "archlinux label");
    }

    #[test]
    fn explicit_request_wins_over_selection_without_changing_it() {
        let r = ready(vec![
            distro("Debian", DistroState::Stopped, DistroKind::Regular),
            distro("archlinux", DistroState::Running, DistroKind::Regular),
        ]);
        assert_eq!(resolve_target(&r, Some("Debian"), Some("archlinux")).unwrap().name, "Debian");
    }

    #[test]
    fn matching_is_case_insensitive_but_returns_the_registered_name() {
        let r = ready(vec![distro("Ubuntu-24.04", DistroState::Stopped, DistroKind::Regular)]);
        assert_eq!(resolve_target(&r, Some("ubuntu-24.04"), None).unwrap().name, "Ubuntu-24.04");
    }

    #[test]
    fn nothing_selected_and_nothing_requested_is_not_guessed() {
        let r = ready(vec![distro("Debian", DistroState::Running, DistroKind::Regular)]);
        assert_eq!(resolve_target(&r, None, None), Err(TerminalError::NoSelection));
    }

    #[test]
    fn invalid_names_are_rejected_before_anything_else() {
        let r = ready(vec![distro("Debian", DistroState::Running, DistroKind::Regular)]);
        for bad in ["", "-d", "--status", "a b", "Debian; calc", "x\"y", "a/b", "..", ".hidden", "name\n", &"a".repeat(65)] {
            assert!(
                matches!(resolve_target(&r, Some(bad), None), Err(TerminalError::InvalidDistro { .. })),
                "{bad:?} must be refused"
            );
        }
        // A stale or hand-edited selection is validated the same way.
        assert!(matches!(resolve_target(&r, None, Some("--exec")), Err(TerminalError::InvalidDistro { .. })));
    }

    #[test]
    fn a_removed_distribution_is_reported_with_what_is_available() {
        let r = ready(vec![distro("Debian", DistroState::Stopped, DistroKind::Regular)]);
        match resolve_target(&r, None, Some("archlinux")) {
            Err(TerminalError::DistroNotFound { name, available }) => {
                assert_eq!(name, "archlinux");
                assert_eq!(available, vec!["Debian".to_string()]);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn other_tools_distributions_are_never_opened() {
        let r = ready(vec![distro("docker-desktop", DistroState::Running, DistroKind::Internal)]);
        assert!(matches!(resolve_target(&r, Some("docker-desktop"), None), Err(TerminalError::DistroNotAllowed { .. })));
    }

    #[test]
    fn busy_distributions_are_refused_with_a_reason() {
        let r = ready(vec![distro("Debian", DistroState::Installing, DistroKind::Regular)]);
        assert!(matches!(resolve_target(&r, Some("Debian"), None), Err(TerminalError::DistroBusy { .. })));
    }

    #[test]
    fn wsl_problems_are_explained_not_launched() {
        let d = vec![distro("Debian", DistroState::Running, DistroKind::Regular)];
        for status in [
            RuntimeStatus::NotInstalled,
            RuntimeStatus::RestartRequired,
            RuntimeStatus::UnsupportedHost { reason: "Not Windows.".into() },
            RuntimeStatus::FeatureProblem { code: None, hint: "Virtualisation is disabled.".into() },
            RuntimeStatus::Error { message: "timed out".into() },
        ] {
            let r = report(status.clone(), d.clone());
            let e = resolve_target(&r, Some("Debian"), None).unwrap_err();
            assert!(matches!(e, TerminalError::WslUnavailable { .. }), "{status:?} -> {e:?}");
            assert!(e.to_string().contains("Linux Setup"));
        }
    }

    #[test]
    fn no_distributions_points_to_linux_setup_and_never_installs() {
        let r = report(RuntimeStatus::NoDistributions, vec![]);
        let e = resolve_target(&r, None, Some("archlinux")).unwrap_err();
        assert_eq!(e, TerminalError::NoDistributions);
        assert!(e.to_string().contains("Linux Setup"));
    }

    #[test]
    fn launch_plan_uses_wsl_exe_with_fixed_arguments() {
        let t = Target { name: "Debian".into(), label: "Debian GNU/Linux".into() };
        let plan = build_plan(&Programs::fake(), &t).unwrap();
        assert_eq!(plan.attempts.len(), 2);
        let first = &plan.attempts[0].spec;
        assert_eq!(first.program, "wsl.exe");
        assert_eq!(first.args, ["-d", "Debian", "--cd", "~"]);
        // The default shell comes from the distribution, not from VLINUX.
        assert!(!first.args.iter().any(|a| a.contains("bash") || a.contains("zsh")));
        let second = &plan.attempts[1].spec;
        assert_eq!(second.args, ["-d", "Debian", "--cd", "~", "--exec", "/bin/sh", "-l"]);
    }

    #[test]
    fn launch_plan_never_elevates_or_changes_wsl_defaults() {
        let t = Target { name: "archlinux".into(), label: "Arch".into() };
        let plan = build_plan(&Programs::fake(), &t).unwrap();
        for a in &plan.attempts {
            for forbidden in ["-u", "--user", "root", "--set-default", "-s", "--unregister", "--shutdown", "--terminate", "--import", "--export"] {
                assert!(!a.spec.args.iter().any(|x| x == forbidden), "{forbidden} must never appear: {:?}", a.spec.args);
            }
        }
    }

    #[test]
    fn a_hostile_name_cannot_become_part_of_a_command_line() {
        let t = Target { name: "x -u root".into(), label: "evil".into() };
        assert!(build_plan(&Programs::fake(), &t).is_err());
        assert!(!is_allowed_session_args(&["-d".into(), "Debian".into(), "-u".into(), "root".into()]));
        assert!(!is_allowed_session_args(&["-d".into(), "Debian".into(), "--cd".into(), "~".into(), "--exec".into(), "rm".into()]));
    }

    #[test]
    fn terminal_environment_is_forwarded_through_wslenv_without_losing_user_entries() {
        assert_eq!(merged_wslenv(None), "TERM:COLORTERM");
        assert_eq!(merged_wslenv(Some("GOPATH/l:TERM".into())), "GOPATH/l:TERM:COLORTERM");
        assert_eq!(merged_wslenv(Some("TERM/u:COLORTERM".into())), "TERM/u:COLORTERM");
    }
}
