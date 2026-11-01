//! Distribution adapters.
//!
//! **VLINUX runs distributions; it does not rebuild them.** Everything that is specific to one
//! distribution family lives in a [`DistroAdapter`]; the rest of the code asks the
//! [`DistroRegistry`] and never compares distribution names itself. Supporting another
//! distribution means adding an adapter (and registering it), not touching the application.
//!
//! M2 scope: identification, package-manager identity and install-source matching. Package
//! installation/removal/update, terminal launching and system information are deliberately
//! *not* implemented yet (M3–M5); the trait is where they will be added.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageManager {
    Apt,
    Pacman,
    Dnf,
    Unknown,
}

impl PackageManager {
    pub fn display_name(&self) -> &'static str {
        match self {
            PackageManager::Apt => "APT",
            PackageManager::Pacman => "pacman",
            PackageManager::Dnf => "DNF",
            PackageManager::Unknown => "unknown",
        }
    }
}

/// Serialisable description of an adapter (what the UI sees).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AdapterInfo {
    pub id: String,
    pub display_name: String,
    pub package_manager: PackageManager,
    pub package_manager_name: String,
    pub rolling_release: bool,
    pub notes: String,
}

pub trait DistroAdapter: Send + Sync {
    /// Stable identifier, e.g. `"arch"`.
    fn id(&self) -> &'static str;
    fn display_name(&self) -> &'static str;
    /// Values of `ID=` in `/etc/os-release` this adapter handles (lower case).
    fn os_release_ids(&self) -> &'static [&'static str];
    /// Lower-case prefixes of *registered WSL names* that suggest this distribution.
    fn registered_name_prefixes(&self) -> &'static [&'static str];
    /// Lower-case prefixes of names in `wsl.exe --list --online` that this adapter can install.
    fn wsl_online_prefixes(&self) -> &'static [&'static str];
    fn package_manager(&self) -> PackageManager;
    fn rolling_release(&self) -> bool {
        false
    }
    /// Short factual note shown in the UI.
    fn notes(&self) -> &'static str {
        ""
    }
    /// Lower is better when VLINUX suggests which *existing* distribution to use.
    fn existing_install_rank(&self) -> u8 {
        10
    }

    fn info(&self) -> AdapterInfo {
        AdapterInfo {
            id: self.id().to_string(),
            display_name: self.display_name().to_string(),
            package_manager: self.package_manager(),
            package_manager_name: self.package_manager().display_name().to_string(),
            rolling_release: self.rolling_release(),
            notes: self.notes().to_string(),
        }
    }
    fn matches_registered_name(&self, name: &str) -> bool {
        prefix_match(name, self.registered_name_prefixes())
    }
    fn matches_wsl_online_name(&self, name: &str) -> bool {
        prefix_match(name, self.wsl_online_prefixes())
    }
}

/// `prefix` must be followed by the end of the name or a non-letter ("ubuntu" matches
/// "Ubuntu-24.04" but "arch" does not match "archive").
fn prefix_match(name: &str, prefixes: &[&str]) -> bool {
    let lc = name.to_ascii_lowercase();
    prefixes.iter().any(|p| match lc.strip_prefix(p) {
        Some(rest) => rest.chars().next().map_or(true, |c| !c.is_ascii_alphabetic()),
        None => false,
    })
}

pub struct DebianAdapter;
impl DistroAdapter for DebianAdapter {
    fn id(&self) -> &'static str {
        "debian"
    }
    fn display_name(&self) -> &'static str {
        "Debian"
    }
    fn os_release_ids(&self) -> &'static [&'static str] {
        &["debian"]
    }
    fn registered_name_prefixes(&self) -> &'static [&'static str] {
        &["debian"]
    }
    fn wsl_online_prefixes(&self) -> &'static [&'static str] {
        &["debian"]
    }
    fn package_manager(&self) -> PackageManager {
        PackageManager::Apt
    }
}

pub struct UbuntuAdapter;
impl DistroAdapter for UbuntuAdapter {
    fn id(&self) -> &'static str {
        "ubuntu"
    }
    fn display_name(&self) -> &'static str {
        "Ubuntu"
    }
    fn os_release_ids(&self) -> &'static [&'static str] {
        &["ubuntu"]
    }
    fn registered_name_prefixes(&self) -> &'static [&'static str] {
        &["ubuntu"]
    }
    fn wsl_online_prefixes(&self) -> &'static [&'static str] {
        &["ubuntu"]
    }
    fn package_manager(&self) -> PackageManager {
        PackageManager::Apt
    }
}

pub struct ArchAdapter;
impl DistroAdapter for ArchAdapter {
    fn id(&self) -> &'static str {
        "arch"
    }
    fn display_name(&self) -> &'static str {
        "Arch Linux"
    }
    fn os_release_ids(&self) -> &'static [&'static str] {
        &["arch"]
    }
    fn registered_name_prefixes(&self) -> &'static [&'static str] {
        &["arch", "archlinux"]
    }
    fn wsl_online_prefixes(&self) -> &'static [&'static str] {
        &["archlinux"]
    }
    fn package_manager(&self) -> PackageManager {
        PackageManager::Pacman
    }
    fn rolling_release(&self) -> bool {
        true
    }
    fn notes(&self) -> &'static str {
        "Rolling release. Always update the whole system before installing packages; partial upgrades are unsupported on Arch."
    }
    /// The product's reference setup: offer an existing Arch first.
    fn existing_install_rank(&self) -> u8 {
        0
    }
}

pub struct FedoraAdapter;
impl DistroAdapter for FedoraAdapter {
    fn id(&self) -> &'static str {
        "fedora"
    }
    fn display_name(&self) -> &'static str {
        "Fedora"
    }
    fn os_release_ids(&self) -> &'static [&'static str] {
        &["fedora"]
    }
    fn registered_name_prefixes(&self) -> &'static [&'static str] {
        &["fedora", "fedoralinux"]
    }
    fn wsl_online_prefixes(&self) -> &'static [&'static str] {
        &["fedoralinux", "fedora"]
    }
    fn package_manager(&self) -> PackageManager {
        PackageManager::Dnf
    }
}

/// Fallback for anything without a dedicated adapter.
pub struct GenericAdapter;
impl DistroAdapter for GenericAdapter {
    fn id(&self) -> &'static str {
        "generic"
    }
    fn display_name(&self) -> &'static str {
        "Linux (no adapter)"
    }
    fn os_release_ids(&self) -> &'static [&'static str] {
        &[]
    }
    fn registered_name_prefixes(&self) -> &'static [&'static str] {
        &[]
    }
    fn wsl_online_prefixes(&self) -> &'static [&'static str] {
        &[]
    }
    fn package_manager(&self) -> PackageManager {
        PackageManager::Unknown
    }
    fn notes(&self) -> &'static str {
        "VLINUX has no adapter for this distribution, so there are no distribution-specific features."
    }
    fn existing_install_rank(&self) -> u8 {
        100
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKind {
    /// `ID=` matched an adapter directly.
    Exact,
    /// Matched through `ID_LIKE` (a derivative, e.g. EndeavourOS -> Arch).
    Derivative,
    /// No adapter matched.
    Fallback,
}

pub struct Resolution<'a> {
    pub adapter: &'a dyn DistroAdapter,
    pub kind: MatchKind,
}

pub struct DistroRegistry {
    adapters: Vec<Box<dyn DistroAdapter>>,
    generic: GenericAdapter,
}

impl Default for DistroRegistry {
    fn default() -> Self {
        Self::with_builtin()
    }
}

impl DistroRegistry {
    pub fn empty() -> Self {
        Self { adapters: Vec::new(), generic: GenericAdapter }
    }

    /// Debian, Ubuntu, Arch and Fedora.
    pub fn with_builtin() -> Self {
        let mut r = Self::empty();
        r.register(Box::new(ArchAdapter));
        r.register(Box::new(DebianAdapter));
        r.register(Box::new(UbuntuAdapter));
        r.register(Box::new(FedoraAdapter));
        r
    }

    /// Add support for another distribution. A later registration with the same id replaces
    /// the earlier one.
    pub fn register(&mut self, adapter: Box<dyn DistroAdapter>) {
        self.adapters.retain(|a| a.id() != adapter.id());
        self.adapters.push(adapter);
    }

    pub fn iter(&self) -> impl Iterator<Item = &dyn DistroAdapter> {
        self.adapters.iter().map(|a| a.as_ref())
    }

    pub fn by_id(&self, id: &str) -> Option<&dyn DistroAdapter> {
        self.iter().find(|a| a.id() == id)
    }

    pub fn generic(&self) -> &dyn DistroAdapter {
        &self.generic
    }

    /// Best guess from the registered WSL name alone (no process is launched).
    pub fn from_registered_name(&self, name: &str) -> &dyn DistroAdapter {
        self.iter().find(|a| a.matches_registered_name(name)).unwrap_or(&self.generic)
    }

    /// Authoritative resolution from `/etc/os-release`: `ID` first, then each `ID_LIKE` entry.
    pub fn from_os_release(&self, id: &str, id_like: &[String]) -> Resolution<'_> {
        let id = id.to_ascii_lowercase();
        if let Some(a) = self.iter().find(|a| a.os_release_ids().contains(&id.as_str())) {
            return Resolution { adapter: a, kind: MatchKind::Exact };
        }
        for like in id_like {
            let like = like.to_ascii_lowercase();
            if let Some(a) = self.iter().find(|a| a.os_release_ids().contains(&like.as_str())) {
                return Resolution { adapter: a, kind: MatchKind::Derivative };
            }
        }
        Resolution { adapter: &self.generic, kind: MatchKind::Fallback }
    }

    /// Adapter able to install a name from `wsl.exe --list --online`.
    pub fn from_wsl_online_name(&self, name: &str) -> Option<&dyn DistroAdapter> {
        self.iter().find(|a| a.matches_wsl_online_name(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_names() {
        let r = DistroRegistry::with_builtin();
        for (name, id) in [
            ("archlinux", "arch"),
            ("Arch", "arch"),
            ("Ubuntu", "ubuntu"),
            ("Ubuntu-24.04", "ubuntu"),
            ("Debian", "debian"),
            ("FedoraLinux-42", "fedora"),
            ("kali-linux", "generic"),
            ("archive", "generic"),
            ("MyDistro", "generic"),
        ] {
            assert_eq!(r.from_registered_name(name).id(), id, "{name}");
        }
    }

    #[test]
    fn os_release_resolution_prefers_id_then_id_like() {
        let r = DistroRegistry::with_builtin();
        let a = r.from_os_release("arch", &[]);
        assert_eq!((a.adapter.id(), a.kind), ("arch", MatchKind::Exact));
        let e = r.from_os_release("endeavouros", &["arch".to_string()]);
        assert_eq!((e.adapter.id(), e.kind), ("arch", MatchKind::Derivative));
        let p = r.from_os_release("pop", &["ubuntu".to_string(), "debian".to_string()]);
        assert_eq!((p.adapter.id(), p.kind), ("ubuntu", MatchKind::Derivative));
        let k = r.from_os_release("alpine", &[]);
        assert_eq!((k.adapter.id(), k.kind), ("generic", MatchKind::Fallback));
    }

    #[test]
    fn package_managers() {
        let r = DistroRegistry::with_builtin();
        assert_eq!(r.by_id("debian").unwrap().package_manager(), PackageManager::Apt);
        assert_eq!(r.by_id("ubuntu").unwrap().package_manager(), PackageManager::Apt);
        assert_eq!(r.by_id("arch").unwrap().package_manager(), PackageManager::Pacman);
        assert_eq!(r.by_id("fedora").unwrap().package_manager(), PackageManager::Dnf);
        assert_eq!(r.generic().package_manager(), PackageManager::Unknown);
    }

    #[test]
    fn online_names() {
        let r = DistroRegistry::with_builtin();
        assert_eq!(r.from_wsl_online_name("Ubuntu-24.04").map(|a| a.id()), Some("ubuntu"));
        assert_eq!(r.from_wsl_online_name("archlinux").map(|a| a.id()), Some("arch"));
        assert_eq!(r.from_wsl_online_name("FedoraLinux-43").map(|a| a.id()), Some("fedora"));
        assert_eq!(r.from_wsl_online_name("Debian").map(|a| a.id()), Some("debian"));
        assert!(r.from_wsl_online_name("kali-linux").is_none());
    }

    struct Alpine;
    impl DistroAdapter for Alpine {
        fn id(&self) -> &'static str {
            "alpine"
        }
        fn display_name(&self) -> &'static str {
            "Alpine"
        }
        fn os_release_ids(&self) -> &'static [&'static str] {
            &["alpine"]
        }
        fn registered_name_prefixes(&self) -> &'static [&'static str] {
            &["alpine"]
        }
        fn wsl_online_prefixes(&self) -> &'static [&'static str] {
            &["alpine"]
        }
        fn package_manager(&self) -> PackageManager {
            PackageManager::Unknown
        }
    }

    #[test]
    fn a_new_distribution_is_supported_by_registering_an_adapter_only() {
        let mut r = DistroRegistry::with_builtin();
        assert_eq!(r.from_registered_name("Alpine").id(), "generic");
        r.register(Box::new(Alpine));
        assert_eq!(r.from_registered_name("Alpine").id(), "alpine");
        assert_eq!(r.from_os_release("alpine", &[]).adapter.id(), "alpine");
        // Re-registering the same id replaces rather than duplicates.
        r.register(Box::new(Alpine));
        assert_eq!(r.iter().filter(|a| a.id() == "alpine").count(), 1);
    }

    #[test]
    fn arch_is_ranked_first_for_existing_installs() {
        let r = DistroRegistry::with_builtin();
        let arch = r.by_id("arch").unwrap().existing_install_rank();
        assert!(r.iter().filter(|a| a.id() != "arch").all(|a| a.existing_install_rank() > arch));
    }
}
