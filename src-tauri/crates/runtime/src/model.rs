//! Data types shared between the runtime layer and the UI (serialised as JSON).

use serde::{Deserialize, Serialize};

use crate::adapters::AdapterInfo;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DistroState {
    Running,
    Stopped,
    Installing,
    Uninstalling,
    Converting,
    /// A state word VLINUX does not recognise (for example a localised one).
    Unknown,
}

/// Distributions VLINUX must never query, select or touch (they belong to other tools).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DistroKind {
    Regular,
    /// e.g. Docker Desktop's / Rancher Desktop's private WSL distributions.
    Internal,
}

/// Where a distribution came from. Existing distributions are never modified by VLINUX.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// Already on the machine before / independently of VLINUX.
    Existing,
    /// Installed by VLINUX with the user's explicit approval.
    InstalledByVlinux,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentitySource {
    /// Guessed from the registered WSL name only (nothing was launched).
    RegisteredName,
    /// Read from `/etc/os-release` inside the distribution.
    OsRelease,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OsRelease {
    pub id: String,
    pub id_like: Vec<String>,
    pub name: Option<String>,
    pub pretty_name: Option<String>,
    pub version: Option<String>,
    pub version_id: Option<String>,
    pub variant_id: Option<String>,
    pub build_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Distro {
    pub name: String,
    pub state: DistroState,
    pub wsl_version: u8,
    pub is_default: bool,
    pub kind: DistroKind,
    pub origin: Origin,
    pub adapter: AdapterInfo,
    pub identity_source: IdentitySource,
    pub os_release: Option<OsRelease>,
    /// Human label, e.g. "Arch Linux" (from os-release when known, else the adapter name).
    pub display_label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HostEnv {
    pub is_windows: bool,
    /// Windows build number, e.g. 22631, when known.
    pub windows_build: Option<u32>,
}

/// Overall state of the WSL runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuntimeStatus {
    /// Not Windows, or a Windows version too old for WSL 2.
    UnsupportedHost { reason: String },
    /// WSL is not installed (or its optional Windows components are not enabled).
    NotInstalled,
    /// WSL (or a Windows feature it needs) is waiting for a restart.
    RestartRequired,
    /// A recognised, actionable problem (virtualisation disabled, missing component, ...).
    FeatureProblem { code: Option<String>, hint: String },
    /// WSL works, but no distribution is registered.
    NoDistributions,
    /// WSL works and at least one distribution is registered.
    Ready,
    /// WSL did not answer sensibly (failure, timeout, unexpected output).
    Error { message: String },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WslReport {
    pub status: RuntimeStatus,
    pub host: HostEnv,
    pub wsl_version: Option<String>,
    pub kernel_version: Option<String>,
    pub wslg_version: Option<String>,
    pub default_wsl_version: Option<u8>,
    /// `Some(true)`: evidence that WSL 2 works. `Some(false)`: ruled out. `None`: cannot tell
    /// (virtualisation is only proven by actually starting a WSL 2 distribution).
    pub wsl2_available: Option<bool>,
    pub distros: Vec<Distro>,
    pub default_distro: Option<String>,
    /// Existing distribution VLINUX suggests using (adapter-ranked); never auto-selected.
    pub suggested_distro: Option<String>,
    /// Result of the Windows "restart pending" probe. `None` = not probed (WSL is healthy).
    pub pending_reboot: Option<bool>,
    pub warnings: Vec<String>,
    pub detected_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatalogEntry {
    /// Name to pass to `wsl.exe --install` (exactly as listed by `wsl.exe --list --online`).
    pub wsl_name: String,
    pub friendly_name: String,
    /// A VLINUX adapter exists for it.
    pub supported: bool,
    pub adapter: Option<AdapterInfo>,
    pub already_installed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Catalog {
    pub entries: Vec<CatalogEntry>,
    /// Set when the official list could not be obtained (offline, timeout, ...).
    pub error: Option<String>,
    pub fetched_at: u64,
}
