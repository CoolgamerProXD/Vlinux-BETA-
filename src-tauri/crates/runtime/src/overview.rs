//! The Distribution Manager's view model (M5): one card per registered distribution, built from
//! the cached WSL report, the persisted inspections and the in-flight operations. Pure data: no
//! process is started here. Nothing is shown that was not read from WSL or from an inspection.
//!
//! **Preferred** is VLINUX's own choice of distribution (`runtime.selected_distro`). It is not the
//! Windows default (`wsl --set-default` is never run), changing it never touches a running terminal,
//! and if the preferred distribution goes missing the user is told: VLINUX never switches silently.

use std::collections::HashMap;

use serde::Serialize;

use crate::adapters::{AdapterInfo, DistroRegistry};
use crate::inspect::Inspection;
use crate::lifecycle::OpKind;
use crate::model::{DistroKind, DistroState, IdentitySource, Origin, RuntimeStatus, WslReport};
use crate::packages::{Availability, PackageCaps};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CardState {
    /// Registered with WSL; its run state is not one VLINUX recognises.
    Detected,
    Running,
    Stopped,
    Starting,
    Installing,
    Inspecting,
    /// Cannot be used from VLINUX (another application's distribution, or WSL is changing it).
    Unavailable,
    /// The last operation on it failed (see `failure`).
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    OpenTerminal,
    OpenFiles,
    Start,
    Stop,
    Inspect,
    SetPreferred,
    Refresh,
    OpenConfig,
    ViewDiagnostics,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ActionState {
    pub action: Action,
    pub enabled: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DistroCard {
    pub name: String,
    pub display_name: String,
    pub state: CardState,
    pub wsl_state: DistroState,
    pub wsl_version: u8,
    pub is_windows_default: bool,
    pub is_preferred: bool,
    pub kind: DistroKind,
    pub origin: Origin,
    pub adapter: AdapterInfo,
    pub identity_source: IdentitySource,
    pub os_pretty: Option<String>,
    pub os_version: Option<String>,
    pub kernel: Option<String>,
    pub arch: Option<String>,
    pub default_user: Option<String>,
    pub packages: PackageCaps,
    pub package_notes: Vec<String>,
    pub inspected_at: Option<u64>,
    pub busy: Option<OpKind>,
    pub failure: Option<String>,
    pub actions: Vec<ActionState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PreferredStatus {
    /// Nothing chosen yet.
    None,
    Ok {
        name: String,
    },
    /// Chosen earlier but no longer registered with WSL.
    Missing {
        name: String,
    },
    /// Registered, but it belongs to another application.
    Internal {
        name: String,
    },
    /// Registered, but WSL is installing / converting / removing it.
    Busy {
        name: String,
    },
    /// WSL is not ready, so the choice cannot be checked.
    Unchecked {
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Overview {
    pub runtime: RuntimeStatus,
    pub cards: Vec<DistroCard>,
    pub preferred: PreferredStatus,
    pub preferred_name: Option<String>,
    /// The Windows default as WSL reports it. VLINUX never changes it.
    pub windows_default: Option<String>,
    /// Plain-language warning about the preferred distribution, if there is one.
    pub warning: Option<String>,
    pub generated_at: u64,
}

pub fn preferred_status(report: &WslReport, preferred: Option<&str>) -> PreferredStatus {
    let Some(name) = preferred else {
        return PreferredStatus::None;
    };
    let name = name.to_string();
    if report.status != RuntimeStatus::Ready {
        return PreferredStatus::Unchecked { name };
    }
    match report.distros.iter().find(|d| d.name == name) {
        None => PreferredStatus::Missing { name },
        Some(d) if d.kind == DistroKind::Internal => PreferredStatus::Internal { name },
        Some(d)
            if matches!(
                d.state,
                DistroState::Installing | DistroState::Uninstalling | DistroState::Converting
            ) =>
        {
            PreferredStatus::Busy { name }
        }
        Some(_) => PreferredStatus::Ok { name },
    }
}

fn warning_for(p: &PreferredStatus, windows_default: Option<&str>) -> Option<String> {
    match p {
        PreferredStatus::Missing { name } => Some(format!(
            "Your preferred distribution '{name}' is no longer registered with WSL. VLINUX has not switched to another one{} — choose a distribution yourself.",
            match windows_default {
                Some(d) => format!(" (the Windows default, '{d}', is not used automatically either)"),
                None => String::new(),
            }
        )),
        PreferredStatus::Internal { name } => Some(format!("'{name}' belongs to another application and cannot be used as your preferred distribution.")),
        PreferredStatus::Busy { name } => Some(format!("Your preferred distribution '{name}' is being changed by WSL right now.")),
        PreferredStatus::Unchecked { name } => Some(format!("WSL is not ready, so VLINUX cannot check that '{name}' still exists.")),
        PreferredStatus::None | PreferredStatus::Ok { .. } => None,
    }
}

fn act(action: Action, enabled: bool, reason: &str) -> ActionState {
    ActionState {
        action,
        enabled,
        reason: if enabled || reason.is_empty() {
            None
        } else {
            Some(reason.to_string())
        },
    }
}

fn actions_for(
    kind: DistroKind,
    wsl_state: DistroState,
    busy: Option<OpKind>,
    is_preferred: bool,
) -> Vec<ActionState> {
    use Action::*;
    let all = [
        OpenTerminal,
        OpenFiles,
        Start,
        Stop,
        Inspect,
        SetPreferred,
        Refresh,
        OpenConfig,
        ViewDiagnostics,
    ];
    if kind == DistroKind::Internal {
        return all
            .iter()
            .map(|a| {
                act(
                    *a,
                    *a == Refresh,
                    "This distribution belongs to another application; VLINUX never touches it.",
                )
            })
            .collect();
    }
    let changing = matches!(
        wsl_state,
        DistroState::Installing | DistroState::Uninstalling | DistroState::Converting
    );
    all.iter()
        .map(|a| {
            if *a == Refresh {
                return act(*a, true, "");
            }
            if changing {
                return act(*a, false, "WSL is changing this distribution right now.");
            }
            if let Some(b) = busy {
                return act(*a, false, &format!("It is {} right now.", b.verb()));
            }
            match a {
                Start => act(*a, wsl_state != DistroState::Running, "Already running."),
                Stop => act(
                    *a,
                    wsl_state == DistroState::Running,
                    "Only a running distribution can be stopped.",
                ),
                SetPreferred => act(*a, !is_preferred, "Already your preferred distribution."),
                _ => act(*a, true, ""),
            }
        })
        .collect()
}

pub fn build_overview(
    report: &WslReport,
    preferred: Option<&str>,
    inspections: &[Inspection],
    ops: &[(String, OpKind)],
    failures: &HashMap<String, String>,
    registry: &DistroRegistry,
    now: u64,
) -> Overview {
    let status = preferred_status(report, preferred);
    let cards = report
        .distros
        .iter()
        .map(|d| {
            let insp = inspections.iter().find(|i| i.distro == d.name);
            let busy = ops.iter().find(|(n, _)| *n == d.name).map(|(_, k)| *k);
            let failure = failures.get(&d.name).cloned();
            let adapter = registry
                .by_id(&d.adapter.id)
                .unwrap_or_else(|| registry.generic());
            let (packages, package_notes) = match insp {
                Some(i) => i.package_caps(adapter.packages()),
                None => (
                    adapter.packages().capabilities(Availability::NotChecked),
                    Vec::new(),
                ),
            };
            let os = insp
                .and_then(|i| i.os_release.as_ref())
                .or(d.os_release.as_ref());
            let state = if d.kind == DistroKind::Internal {
                CardState::Unavailable
            } else if let Some(b) = busy {
                match b {
                    OpKind::Start => CardState::Starting,
                    OpKind::Inspect => CardState::Inspecting,
                    OpKind::Stop => state_from_wsl(d.state),
                }
            } else {
                match d.state {
                    DistroState::Installing => CardState::Installing,
                    DistroState::Uninstalling | DistroState::Converting => CardState::Unavailable,
                    _ if failure.is_some() => CardState::Failed,
                    other => state_from_wsl(other),
                }
            };
            let is_preferred = preferred == Some(d.name.as_str());
            DistroCard {
                name: d.name.clone(),
                display_name: d.display_label.clone(),
                state,
                wsl_state: d.state,
                wsl_version: d.wsl_version,
                is_windows_default: d.is_default,
                is_preferred,
                kind: d.kind,
                origin: d.origin,
                adapter: d.adapter.clone(),
                identity_source: d.identity_source,
                os_pretty: os.and_then(|o| o.pretty_name.clone().or_else(|| o.name.clone())),
                os_version: os.and_then(|o| o.version_id.clone().or_else(|| o.version.clone())),
                kernel: insp.and_then(|i| i.kernel.clone()),
                arch: insp.and_then(|i| i.arch.clone()),
                default_user: insp.and_then(|i| i.default_user.clone()),
                packages,
                package_notes,
                inspected_at: insp.map(|i| i.at),
                busy,
                failure,
                actions: actions_for(d.kind, d.state, busy, is_preferred),
            }
        })
        .collect();
    Overview {
        runtime: report.status.clone(),
        cards,
        warning: warning_for(&status, report.default_distro.as_deref()),
        preferred: status,
        preferred_name: preferred.map(|s| s.to_string()),
        windows_default: report.default_distro.clone(),
        generated_at: now,
    }
}

fn state_from_wsl(s: DistroState) -> CardState {
    match s {
        DistroState::Running => CardState::Running,
        DistroState::Stopped => CardState::Stopped,
        DistroState::Installing => CardState::Installing,
        DistroState::Uninstalling | DistroState::Converting => CardState::Unavailable,
        DistroState::Unknown => CardState::Detected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DistroKind::{Internal, Regular};
    use crate::model::DistroState::{Converting, Installing, Running, Stopped, Unknown};
    use crate::testing::fake_report;

    fn report() -> WslReport {
        fake_report(
            &[
                ("archlinux", Running, Regular, 2),
                ("Ubuntu-24.04", Stopped, Regular, 2),
                ("Debian", Running, Regular, 1),
                ("Mystery", Unknown, Regular, 2),
                ("Fedora", Installing, Regular, 2),
                ("Old", Converting, Regular, 2),
                ("docker-desktop", Running, Internal, 2),
            ],
            Some("archlinux"),
        )
    }

    fn build(
        preferred: Option<&str>,
        ops: &[(String, OpKind)],
        failures: &HashMap<String, String>,
    ) -> Overview {
        build_overview(
            &report(),
            preferred,
            &[],
            ops,
            failures,
            &DistroRegistry::with_builtin(),
            99,
        )
    }

    fn card<'a>(o: &'a Overview, n: &str) -> &'a DistroCard {
        o.cards.iter().find(|c| c.name == n).unwrap()
    }

    fn enabled(c: &DistroCard, a: Action) -> bool {
        c.actions.iter().find(|x| x.action == a).unwrap().enabled
    }

    #[test]
    fn states_come_from_wsl_and_never_invent() {
        let o = build(None, &[], &HashMap::new());
        assert_eq!(card(&o, "archlinux").state, CardState::Running);
        assert_eq!(card(&o, "Ubuntu-24.04").state, CardState::Stopped);
        assert_eq!(card(&o, "Mystery").state, CardState::Detected);
        assert_eq!(card(&o, "Fedora").state, CardState::Installing);
        assert_eq!(card(&o, "Old").state, CardState::Unavailable);
        assert_eq!(card(&o, "docker-desktop").state, CardState::Unavailable);
        assert_eq!(card(&o, "Debian").wsl_version, 1);
        // Nothing was inspected: no kernel, user or version is shown.
        let c = card(&o, "archlinux");
        assert!(
            c.kernel.is_none()
                && c.default_user.is_none()
                && c.arch.is_none()
                && c.inspected_at.is_none()
        );
        assert_eq!(c.packages.availability, Availability::NotChecked);
    }

    #[test]
    fn preferred_is_independent_of_the_windows_default() {
        let o = build(Some("Ubuntu-24.04"), &[], &HashMap::new());
        assert_eq!(o.windows_default.as_deref(), Some("archlinux"));
        assert_eq!(
            o.preferred,
            PreferredStatus::Ok {
                name: "Ubuntu-24.04".into()
            }
        );
        assert!(
            card(&o, "Ubuntu-24.04").is_preferred && !card(&o, "Ubuntu-24.04").is_windows_default
        );
        assert!(card(&o, "archlinux").is_windows_default && !card(&o, "archlinux").is_preferred);
        assert!(o.warning.is_none());
    }

    #[test]
    fn missing_preferred_warns_and_never_switches() {
        let o = build(Some("Gone"), &[], &HashMap::new());
        assert_eq!(
            o.preferred,
            PreferredStatus::Missing {
                name: "Gone".into()
            }
        );
        assert_eq!(o.preferred_name.as_deref(), Some("Gone"));
        assert!(o.cards.iter().all(|c| !c.is_preferred));
        let w = o.warning.unwrap();
        assert!(w.contains("Gone") && w.contains("has not switched"));
    }

    #[test]
    fn internal_busy_and_not_ready_preferred() {
        assert!(matches!(
            build(Some("docker-desktop"), &[], &HashMap::new()).preferred,
            PreferredStatus::Internal { .. }
        ));
        assert!(matches!(
            build(Some("Fedora"), &[], &HashMap::new()).preferred,
            PreferredStatus::Busy { .. }
        ));
        let mut r = report();
        r.status = RuntimeStatus::NotInstalled;
        assert!(matches!(
            preferred_status(&r, Some("archlinux")),
            PreferredStatus::Unchecked { .. }
        ));
        assert_eq!(preferred_status(&r, None), PreferredStatus::None);
    }

    #[test]
    fn actions_follow_state() {
        let o = build(Some("archlinux"), &[], &HashMap::new());
        let run = card(&o, "archlinux");
        assert!(
            !enabled(run, Action::Start)
                && enabled(run, Action::Stop)
                && enabled(run, Action::OpenTerminal)
        );
        assert!(!enabled(run, Action::SetPreferred), "already preferred");
        let stopped = card(&o, "Ubuntu-24.04");
        assert!(
            enabled(stopped, Action::Start)
                && !enabled(stopped, Action::Stop)
                && enabled(stopped, Action::SetPreferred)
        );
        let internal = card(&o, "docker-desktop");
        assert!(internal
            .actions
            .iter()
            .all(|a| a.action == Action::Refresh || !a.enabled));
        assert!(internal
            .actions
            .iter()
            .find(|a| a.action == Action::Start)
            .unwrap()
            .reason
            .is_some());
        let changing = card(&o, "Fedora");
        assert!(changing
            .actions
            .iter()
            .all(|a| a.action == Action::Refresh || !a.enabled));
    }

    #[test]
    fn in_flight_operations_set_state_and_block_actions() {
        let ops = vec![
            ("Ubuntu-24.04".to_string(), OpKind::Start),
            ("Debian".to_string(), OpKind::Inspect),
            ("archlinux".to_string(), OpKind::Stop),
        ];
        let o = build(None, &ops, &HashMap::new());
        assert_eq!(card(&o, "Ubuntu-24.04").state, CardState::Starting);
        assert_eq!(card(&o, "Debian").state, CardState::Inspecting);
        assert_eq!(card(&o, "archlinux").state, CardState::Running);
        assert!(!enabled(card(&o, "Ubuntu-24.04"), Action::Start));
        assert!(!enabled(card(&o, "archlinux"), Action::Stop));
        assert!(enabled(card(&o, "archlinux"), Action::Refresh));
    }

    #[test]
    fn failures_show_as_failed_until_cleared() {
        let mut f = HashMap::new();
        f.insert(
            "Ubuntu-24.04".to_string(),
            "WSL could not start it".to_string(),
        );
        let o = build(None, &[], &f);
        let c = card(&o, "Ubuntu-24.04");
        assert_eq!(c.state, CardState::Failed);
        assert_eq!(c.failure.as_deref(), Some("WSL could not start it"));
        assert!(enabled(c, Action::Start), "retry stays possible");
    }

    #[test]
    fn inspection_data_is_shown_with_its_timestamp() {
        let insp = Inspection {
            distro: "archlinux".into(),
            at: 55,
            was_running: true,
            started_by_inspection: false,
            os_release: Some(crate::osrelease::parse_os_release(
                "NAME=\"Arch Linux\"\nID=arch\nPRETTY_NAME=\"Arch Linux\"\nBUILD_ID=rolling\n",
            )),
            kernel: Some("5.15".into()),
            arch: Some("x86_64".into()),
            default_user: Some("dana".into()),
            uid: Some(1000),
            managers: vec![crate::inspect::ManagerProbe {
                binary: "pacman".into(),
                present: true,
                version: Some("Pacman v7".into()),
            }],
            notes: vec![],
        };
        let o = build_overview(
            &report(),
            None,
            &[insp],
            &[],
            &HashMap::new(),
            &DistroRegistry::with_builtin(),
            99,
        );
        let c = card(&o, "archlinux");
        assert_eq!(c.inspected_at, Some(55));
        assert_eq!(c.kernel.as_deref(), Some("5.15"));
        assert_eq!(c.default_user.as_deref(), Some("dana"));
        assert_eq!(c.os_pretty.as_deref(), Some("Arch Linux"));
        assert!(matches!(
            c.packages.availability,
            Availability::Present { .. }
        ));
        // Other cards are not given this data.
        assert!(card(&o, "Debian").kernel.is_none());
    }
}
