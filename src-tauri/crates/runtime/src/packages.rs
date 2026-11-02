//! Package-manager capabilities (M5).
//!
//! VLINUX does not pretend that APT, pacman and DNF behave alike. Each adapter hands back a
//! [`PackageSupport`]: the binary to look for, how to ask it for its version, how to *search*
//! (read-only), and the exact command lines VLINUX will *show* for installs and updates.
//!
//! What VLINUX actually does:
//! * **detect** and **version**: probe the binary inside the distribution (never assumed from the
//!   distribution's name);
//! * **search**: a typed, validated, read-only query run on explicit request;
//! * **install / update**: *plan only*. Installing changes the Linux system and needs root, and
//!   VLINUX never escalates privileges, so it shows the exact command for the user to review and
//!   run themselves in the Terminal. Nothing is ever run automatically, and no system upgrade is
//!   ever started by VLINUX.

use serde::Serialize;

use crate::adapters::PackageManager;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Support {
    /// VLINUX can do this itself.
    Available,
    /// The package manager can, but VLINUX deliberately only shows the command (see module docs).
    PlanOnly,
    /// Not possible: unknown package manager, or the manager was not found in the distribution.
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Capability {
    pub support: Support,
    pub note: String,
}

impl Capability {
    fn new(support: Support, note: impl Into<String>) -> Self {
        Self { support, note: note.into() }
    }
}

/// What probing the distribution found for this package manager.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Availability {
    /// Not looked at yet (the distribution was not inspected).
    NotChecked,
    Present { version: Option<String> },
    Absent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PackageCaps {
    pub manager: PackageManager,
    pub manager_name: String,
    pub binary: Option<String>,
    pub availability: Availability,
    pub detect: Capability,
    pub version: Capability,
    pub search: Capability,
    pub install: Capability,
    pub update: Capability,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageOp {
    Install,
    Update,
}

/// A reviewable plan. `executed` is always `false`: VLINUX shows it, the user runs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PackagePlan {
    pub distro: String,
    pub op: PackageOp,
    pub manager_name: String,
    pub packages: Vec<String>,
    /// One command per line, exactly as they would be typed in the distribution's terminal.
    pub commands: Vec<String>,
    pub requires_root: bool,
    pub executed: bool,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchStyle {
    AptCache,
    PacmanSs,
    DnfSearch,
}

#[derive(Debug, Clone, Copy)]
pub struct PackageSupport {
    pub manager: PackageManager,
    binary: &'static str,
    search: Option<SearchStyle>,
}

/// Binaries that are looked for (in this order) when the adapter's own manager is missing, so the
/// UI can say "this distribution uses `apk`, which VLINUX has no adapter for".
pub const PROBE_BINARIES: [&str; 5] = ["apt-get", "pacman", "dnf", "apk", "zypper"];

pub const MAX_PACKAGES: usize = 20;
pub const MAX_QUERY: usize = 64;

impl PackageSupport {
    pub const fn none() -> Self {
        Self { manager: PackageManager::Unknown, binary: "", search: None }
    }

    pub fn for_manager(pm: PackageManager) -> Self {
        match pm {
            PackageManager::Apt => Self { manager: pm, binary: "apt-get", search: Some(SearchStyle::AptCache) },
            PackageManager::Pacman => Self { manager: pm, binary: "pacman", search: Some(SearchStyle::PacmanSs) },
            PackageManager::Dnf => Self { manager: pm, binary: "dnf", search: Some(SearchStyle::DnfSearch) },
            PackageManager::Unknown => Self::none(),
        }
    }

    pub fn binary(&self) -> Option<&'static str> {
        if self.binary.is_empty() {
            None
        } else {
            Some(self.binary)
        }
    }

    /// The argv (after `--exec`) of a read-only search, or `None` when search isn't supported.
    /// `query` must already satisfy [`valid_search_query`].
    pub fn search_argv(&self, query: &str) -> Option<Vec<String>> {
        if !valid_search_query(query) {
            return None;
        }
        let v = |parts: &[&str]| -> Vec<String> { parts.iter().map(|s| s.to_string()).chain(std::iter::once(query.to_string())).collect() };
        match self.search? {
            SearchStyle::AptCache => Some(v(&["apt-cache", "search", "--names-only"])),
            SearchStyle::PacmanSs => Some(v(&["pacman", "-Ss"])),
            SearchStyle::DnfSearch => Some(v(&["dnf", "--quiet", "search"])),
        }
    }

    pub fn install_commands(&self, packages: &[String]) -> Option<Vec<String>> {
        let list = packages.join(" ");
        match self.manager {
            PackageManager::Apt => Some(vec![format!("sudo apt install {list}")]),
            PackageManager::Pacman => Some(vec![format!("sudo pacman -S --needed {list}")]),
            PackageManager::Dnf => Some(vec![format!("sudo dnf install {list}")]),
            PackageManager::Unknown => None,
        }
    }

    pub fn update_commands(&self) -> Option<Vec<String>> {
        match self.manager {
            PackageManager::Apt => Some(vec!["sudo apt update".into(), "sudo apt upgrade".into()]),
            PackageManager::Pacman => Some(vec!["sudo pacman -Syu".into()]),
            PackageManager::Dnf => Some(vec!["sudo dnf upgrade --refresh".into()]),
            PackageManager::Unknown => None,
        }
    }

    /// Distribution-family cautions shown with a plan.
    fn plan_notes(&self) -> Vec<String> {
        match self.manager {
            PackageManager::Pacman => vec!["Arch does not support partial upgrades: update the whole system before installing new packages.".into()],
            PackageManager::Apt => vec!["Run `sudo apt update` first if the package lists are old.".into()],
            _ => Vec::new(),
        }
    }

    /// What this distribution's package manager can do, given what probing found.
    pub fn capabilities(&self, availability: Availability) -> PackageCaps {
        let name = self.manager.display_name().to_string();
        let supported_manager = self.manager != PackageManager::Unknown;
        let present = matches!(availability, Availability::Present { .. });
        let checked = !matches!(availability, Availability::NotChecked);

        let detect = if supported_manager {
            Capability::new(Support::Available, "VLINUX looks for the package manager inside the distribution; it is never assumed from the distribution's name.")
        } else {
            Capability::new(Support::Unsupported, "VLINUX has no package-manager adapter for this distribution.")
        };
        let version = match (&availability, supported_manager) {
            (_, false) => Capability::new(Support::Unsupported, "No adapter."),
            (Availability::Present { version: Some(_) }, _) => Capability::new(Support::Available, "Read from the package manager."),
            (Availability::Present { version: None }, _) => Capability::new(Support::Unsupported, "The package manager did not report a version."),
            (Availability::Absent, _) => Capability::new(Support::Unsupported, format!("{name} was not found in this distribution.")),
            (Availability::NotChecked, _) => Capability::new(Support::Unsupported, "Inspect the distribution to find out."),
        };
        let gate = |ok_note: &str, plan: bool| -> Capability {
            if !supported_manager {
                Capability::new(Support::Unsupported, "VLINUX has no adapter for this distribution's package manager.")
            } else if !checked {
                Capability::new(Support::Unsupported, "Inspect the distribution first: availability is checked, not assumed.")
            } else if !present {
                Capability::new(Support::Unsupported, format!("{name} was not found in this distribution."))
            } else if plan {
                Capability::new(Support::PlanOnly, ok_note)
            } else {
                Capability::new(Support::Available, ok_note)
            }
        };
        let search = if self.search.is_none() && supported_manager {
            Capability::new(Support::Unsupported, "Search is not implemented for this package manager.")
        } else {
            gate("Read-only search, run only when you ask. Nothing is installed or changed.", false)
        };
        let install = gate("VLINUX shows the exact command to review and run in your terminal. It never installs packages itself: that needs root, and VLINUX does not escalate privileges.", true);
        let update = gate("VLINUX shows the exact commands to run in your terminal. It never starts a system upgrade on its own.", true);
        PackageCaps {
            manager: self.manager,
            manager_name: name,
            binary: self.binary().map(|b| b.to_string()),
            availability,
            detect,
            version,
            search,
            install,
            update,
        }
    }

    /// Build a reviewable plan. Validates every package name; never executes anything.
    pub fn plan(&self, distro: &str, op: PackageOp, packages: &[String]) -> Result<PackagePlan, String> {
        let commands = match op {
            PackageOp::Install => {
                if packages.is_empty() {
                    return Err("Enter at least one package name.".into());
                }
                if packages.len() > MAX_PACKAGES {
                    return Err(format!("At most {MAX_PACKAGES} packages can be reviewed at once."));
                }
                for p in packages {
                    if !valid_package_name(p) {
                        return Err(format!("'{}' is not a valid package name.", p.chars().take(40).collect::<String>()));
                    }
                }
                self.install_commands(packages)
            }
            PackageOp::Update => self.update_commands(),
        };
        let commands = commands.ok_or_else(|| "VLINUX has no package-manager adapter for this distribution, so it can't prepare a command.".to_string())?;
        let mut notes = self.plan_notes();
        notes.push("VLINUX has not run this. Copy it into the Terminal for this distribution if you want to go ahead; `sudo` will ask for your password there.".into());
        Ok(PackagePlan {
            distro: distro.to_string(),
            op,
            manager_name: self.manager.display_name().to_string(),
            packages: if op == PackageOp::Install { packages.to_vec() } else { Vec::new() },
            commands,
            requires_root: true,
            executed: false,
            notes,
        })
    }
}

/// A package name safe to show in a command and to pass as one argument.
pub fn valid_package_name(p: &str) -> bool {
    let mut chars = p.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    p.len() <= 100 && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '_' | '-'))
}

/// A search term: letters, digits and `+ . _ -`, not starting with `-` (so it can never be an option).
pub fn valid_search_query(q: &str) -> bool {
    !q.is_empty() && q.len() <= MAX_QUERY && valid_package_name(q)
}

/// Pick the first line that names a version out of a package manager's `--version` output.
pub fn parse_version(binary: &str, text: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
    let first = lines.first()?;
    let clean = |s: &str| -> Option<String> {
        let s: String = s.chars().take(80).collect();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    };
    match binary {
        // "apt 2.6.1 (amd64)"
        "apt-get" => clean(first),
        // The banner is ASCII art; the version is on the line containing "Pacman v7.0.0".
        "pacman" => lines.iter().find_map(|l| l.find("Pacman v").map(|i| l[i..].split_whitespace().take(2).collect::<Vec<_>>().join(" "))).and_then(|s| clean(&s)),
        // "4.18.2" then plugin lines
        "dnf" => clean(first),
        // "apk-tools 2.14.4, compiled for x86_64."
        _ => clean(first),
    }
}

/// One search result. Parsed from the package manager's own output; nothing is invented.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PackageHit {
    pub name: String,
    pub version: Option<String>,
    pub description: String,
}

pub const MAX_HITS: usize = 50;

/// Parse a manager's search output into at most [`MAX_HITS`] hits. Unrecognised lines are skipped.
pub fn parse_search(manager: PackageManager, text: &str) -> Vec<PackageHit> {
    let mut hits: Vec<PackageHit> = Vec::new();
    let short = |s: &str, n: usize| -> String { s.trim().chars().take(n).collect() };
    match manager {
        PackageManager::Apt => {
            for line in text.lines() {
                if let Some((name, desc)) = line.split_once(" - ") {
                    let name = name.trim();
                    if valid_package_name(name) {
                        hits.push(PackageHit { name: name.to_string(), version: None, description: short(desc, 200) });
                    }
                }
                if hits.len() >= MAX_HITS {
                    break;
                }
            }
        }
        PackageManager::Pacman => {
            let mut lines = text.lines().peekable();
            while let Some(line) = lines.next() {
                if line.starts_with(char::is_whitespace) || line.trim().is_empty() {
                    continue;
                }
                let mut parts = line.split_whitespace();
                let Some(first) = parts.next() else { continue };
                let name = first.rsplit('/').next().unwrap_or(first);
                if !valid_package_name(name) {
                    continue;
                }
                let version = parts.next().map(|v| short(v, 40));
                let description = match lines.peek() {
                    Some(next) if next.starts_with(char::is_whitespace) => short(lines.next().unwrap_or(""), 200),
                    _ => String::new(),
                };
                hits.push(PackageHit { name: name.to_string(), version, description });
                if hits.len() >= MAX_HITS {
                    break;
                }
            }
        }
        PackageManager::Dnf => {
            for line in text.lines() {
                if line.starts_with('=') || line.starts_with(char::is_whitespace) {
                    continue;
                }
                if let Some((name, desc)) = line.split_once(" : ") {
                    let base = name.trim().rsplit_once('.').map(|(n, _arch)| n).unwrap_or(name.trim());
                    if valid_package_name(base) {
                        hits.push(PackageHit { name: base.to_string(), version: None, description: short(desc, 200) });
                    }
                }
                if hits.len() >= MAX_HITS {
                    break;
                }
            }
        }
        PackageManager::Unknown => {}
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn present() -> Availability {
        Availability::Present { version: Some("1".into()) }
    }

    #[test]
    fn manager_to_binary_and_search() {
        let apt = PackageSupport::for_manager(PackageManager::Apt);
        assert_eq!(apt.binary(), Some("apt-get"));
        assert_eq!(apt.search_argv("git").unwrap(), ["apt-cache", "search", "--names-only", "git"]);
        let pac = PackageSupport::for_manager(PackageManager::Pacman);
        assert_eq!(pac.search_argv("git").unwrap(), ["pacman", "-Ss", "git"]);
        let dnf = PackageSupport::for_manager(PackageManager::Dnf);
        assert_eq!(dnf.search_argv("git").unwrap(), ["dnf", "--quiet", "search", "git"]);
        let none = PackageSupport::for_manager(PackageManager::Unknown);
        assert_eq!(none.binary(), None);
        assert!(none.search_argv("git").is_none());
    }

    #[test]
    fn hostile_queries_never_become_argv() {
        let apt = PackageSupport::for_manager(PackageManager::Apt);
        let long = "x".repeat(65);
        for bad in ["", "-h", "--version", "a b", "a;b", "$(x)", "a|b", "a`b`", "../x", "a\nb", "a\"b", long.as_str()] {
            assert!(apt.search_argv(bad).is_none(), "{bad:?}");
        }
        assert!(apt.search_argv("g++").is_some());
    }

    #[test]
    fn capabilities_are_checked_not_assumed() {
        let apt = PackageSupport::for_manager(PackageManager::Apt);
        let unchecked = apt.capabilities(Availability::NotChecked);
        assert_eq!(unchecked.search.support, Support::Unsupported);
        assert_eq!(unchecked.install.support, Support::Unsupported);
        assert!(unchecked.install.note.contains("Inspect"));
        let absent = apt.capabilities(Availability::Absent);
        assert_eq!(absent.search.support, Support::Unsupported);
        assert!(absent.install.note.contains("not found"));
        let ok = apt.capabilities(present());
        assert_eq!(ok.search.support, Support::Available);
        assert_eq!(ok.install.support, Support::PlanOnly);
        assert_eq!(ok.update.support, Support::PlanOnly);
        assert_eq!(ok.version.support, Support::Available);
    }

    #[test]
    fn unknown_distributions_claim_nothing() {
        let g = PackageSupport::none().capabilities(Availability::NotChecked);
        for c in [&g.detect, &g.version, &g.search, &g.install, &g.update] {
            assert_eq!(c.support, Support::Unsupported);
        }
        assert!(PackageSupport::none().plan("X", PackageOp::Update, &[]).is_err());
    }

    #[test]
    fn plans_show_exact_commands_and_never_execute() {
        let pac = PackageSupport::for_manager(PackageManager::Pacman);
        let p = pac.plan("archlinux", PackageOp::Install, &["git".into(), "base-devel".into()]).unwrap();
        assert_eq!(p.commands, ["sudo pacman -S --needed git base-devel"]);
        assert!(!p.executed && p.requires_root);
        assert!(p.notes.iter().any(|n| n.contains("partial upgrades")));
        let apt = PackageSupport::for_manager(PackageManager::Apt);
        assert_eq!(apt.plan("Debian", PackageOp::Update, &[]).unwrap().commands, ["sudo apt update", "sudo apt upgrade"]);
        let dnf = PackageSupport::for_manager(PackageManager::Dnf);
        assert_eq!(dnf.plan("Fedora", PackageOp::Install, &["gcc".into()]).unwrap().commands, ["sudo dnf install gcc"]);
    }

    #[test]
    fn plans_reject_bad_names_and_empty_or_huge_lists() {
        let apt = PackageSupport::for_manager(PackageManager::Apt);
        assert!(apt.plan("D", PackageOp::Install, &[]).is_err());
        assert!(apt.plan("D", PackageOp::Install, &["git; rm -rf /".into()]).is_err());
        assert!(apt.plan("D", PackageOp::Install, &["-y".into()]).is_err());
        assert!(apt.plan("D", PackageOp::Install, &vec!["a".to_string(); MAX_PACKAGES + 1]).is_err());
    }

    #[test]
    fn version_parsing() {
        assert_eq!(parse_version("apt-get", "apt 2.6.1 (amd64)\nUsing...").as_deref(), Some("apt 2.6.1 (amd64)"));
        let pac = "\n .--.                  Pacman v7.0.0 - libalpm v15.0.0\n/ _.-' .-.  .-.  .-.   Copyright (C) 2006-2024 Pacman Development Team\n";
        assert_eq!(parse_version("pacman", pac).as_deref(), Some("Pacman v7.0.0"));
        assert_eq!(parse_version("dnf", "4.18.2\n  Installed: dnf-0:4.18.2").as_deref(), Some("4.18.2"));
        assert_eq!(parse_version("apt-get", ""), None);
        assert_eq!(parse_version("pacman", "something else"), None);
    }

    #[test]
    fn search_parsing_per_manager() {
        let apt = parse_search(PackageManager::Apt, "git - fast, scalable, distributed revision control system\ngit-lfs - Git Large File Support\nweird line\n");
        assert_eq!(apt.len(), 2);
        assert_eq!(apt[0].name, "git");
        let pac = parse_search(PackageManager::Pacman, "extra/git 2.45.2-1 [installed]\n    the fast distributed version control system\nextra/git-lfs 3.5.1-1\n    Git extension\n");
        assert_eq!(pac.len(), 2);
        assert_eq!(pac[0].name, "git");
        assert_eq!(pac[0].version.as_deref(), Some("2.45.2-1"));
        assert!(pac[1].description.contains("extension"));
        let dnf = parse_search(PackageManager::Dnf, "========== Name Matched: git ==========\ngit.x86_64 : Fast version control\ngit-lfs.x86_64 : Git LFS\n");
        assert_eq!(dnf.iter().map(|h| h.name.as_str()).collect::<Vec<_>>(), ["git", "git-lfs"]);
        assert!(parse_search(PackageManager::Unknown, "a - b").is_empty());
    }

    #[test]
    fn search_results_are_capped() {
        let many: String = (0..200).map(|i| format!("pkg{i} - thing\n")).collect();
        assert_eq!(parse_search(PackageManager::Apt, &many).len(), MAX_HITS);
    }
}
