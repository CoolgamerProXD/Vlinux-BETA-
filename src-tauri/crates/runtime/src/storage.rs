//! Shared Windows/Linux personal-folder storage model (M2 = architecture and groundwork).
//!
//! Intended behaviour (the *application* of mappings arrives with the M4 file manager):
//! `~/Downloads`, `~/Documents`, `~/Pictures`, `~/Videos`, `~/Music` and `~/Desktop` inside the
//! distribution can point **directly** at the user's real Windows folders (a symlink into the
//! `/mnt/<drive>/...` mount) — no copying, no background sync. Everything else (system files,
//! package databases and caches, dotfiles, and by default development projects) stays inside the
//! distribution's private filesystem.
//!
//! Rules this module encodes:
//! * Windows Known Folder locations are resolved **dynamically** (never hard-coded user names,
//!   `C:\Users\...` paths or drive letters), so redirected folders and other drives work.
//! * Locations WSL cannot reach through a normal drive mount (UNC/network shares, `\\wsl$`,
//!   device paths, a whole drive root) are **detected and reported as unsupported**, never mapped
//!   incorrectly.
//! * [`plan_mapping`] never plans a deletion of user data: non-empty directories and files are
//!   handed back to the user; only an *empty* directory or a *symlink* may be replaced, and only
//!   after explicit confirmation.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnownFolder {
    Desktop,
    Documents,
    Downloads,
    Music,
    Pictures,
    Videos,
}

impl KnownFolder {
    pub const ALL: [KnownFolder; 6] = [
        KnownFolder::Downloads,
        KnownFolder::Documents,
        KnownFolder::Pictures,
        KnownFolder::Videos,
        KnownFolder::Music,
        KnownFolder::Desktop,
    ];
    pub fn id(&self) -> &'static str {
        match self {
            KnownFolder::Desktop => "desktop",
            KnownFolder::Documents => "documents",
            KnownFolder::Downloads => "downloads",
            KnownFolder::Music => "music",
            KnownFolder::Pictures => "pictures",
            KnownFolder::Videos => "videos",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            KnownFolder::Desktop => "Desktop",
            KnownFolder::Documents => "Documents",
            KnownFolder::Downloads => "Downloads",
            KnownFolder::Music => "Music",
            KnownFolder::Pictures => "Pictures",
            KnownFolder::Videos => "Videos",
        }
    }
    /// Directory name under the Linux home (`~/Downloads`).
    pub fn linux_dir(&self) -> &'static str {
        self.label()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MappingMode {
    /// Point the Linux folder at the Windows folder.
    #[default]
    Shared,
    /// Keep the folder inside the distribution (not visible to Windows).
    Private,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct FolderConfig {
    pub mode: MappingMode,
    /// `None` = the Windows Known Folder (resolved dynamically). `Some` = a user-chosen Windows
    /// folder (validated before use).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub windows_path: Option<String>,
}

/// Where development projects live. Linux-native storage is recommended: file access through
/// `/mnt/...` is considerably slower for build-heavy workloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProjectsMode {
    #[default]
    LinuxPrivate,
    Shared,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct StorageConfig {
    /// Declared first: TOML wants plain values before tables.
    pub projects: ProjectsMode,
    pub desktop: FolderConfig,
    pub documents: FolderConfig,
    pub downloads: FolderConfig,
    pub music: FolderConfig,
    pub pictures: FolderConfig,
    pub videos: FolderConfig,
}

const MAX_PATH_TEXT: usize = 1024;

impl StorageConfig {
    pub fn folder(&self, f: KnownFolder) -> &FolderConfig {
        match f {
            KnownFolder::Desktop => &self.desktop,
            KnownFolder::Documents => &self.documents,
            KnownFolder::Downloads => &self.downloads,
            KnownFolder::Music => &self.music,
            KnownFolder::Pictures => &self.pictures,
            KnownFolder::Videos => &self.videos,
        }
    }

    pub fn folder_mut(&mut self, f: KnownFolder) -> &mut FolderConfig {
        match f {
            KnownFolder::Desktop => &mut self.desktop,
            KnownFolder::Documents => &mut self.documents,
            KnownFolder::Downloads => &mut self.downloads,
            KnownFolder::Music => &mut self.music,
            KnownFolder::Pictures => &mut self.pictures,
            KnownFolder::Videos => &mut self.videos,
        }
    }

    /// Normalise stored text: trim, drop empty/oversized/NUL-containing custom paths. (Whether a
    /// path is *usable* is decided by [`windows_to_wsl`] and shown to the user, not hidden here.)
    pub fn sanitized(mut self) -> Self {
        for f in KnownFolder::ALL {
            let c = self.folder_mut(f);
            c.windows_path = c
                .windows_path
                .take()
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty() && p.len() <= MAX_PATH_TEXT && !p.contains('\0'));
        }
        self
    }
}

/// Resolves Windows Known Folders (Desktop, Documents, ...) to their *current* location.
pub trait KnownFolderResolver {
    fn resolve(&self, folder: KnownFolder) -> Option<String>;
}

/// Uses the platform's shell folder API (`SHGetKnownFolderPath` on Windows, via the `dirs`
/// crate), so redirected folders and folders on other drives are returned correctly.
pub struct SystemKnownFolders;

impl KnownFolderResolver for SystemKnownFolders {
    fn resolve(&self, folder: KnownFolder) -> Option<String> {
        let p = match folder {
            KnownFolder::Desktop => dirs::desktop_dir(),
            KnownFolder::Documents => dirs::document_dir(),
            KnownFolder::Downloads => dirs::download_dir(),
            KnownFolder::Music => dirs::audio_dir(),
            KnownFolder::Pictures => dirs::picture_dir(),
            KnownFolder::Videos => dirs::video_dir(),
        };
        p.map(|p| p.to_string_lossy().into_owned())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DriveKind {
    Fixed,
    Removable,
    Network,
    Optical,
    Ram,
    Unknown,
}

pub trait DriveInspector {
    fn kind(&self, letter: char) -> DriveKind;
}

pub struct SystemDrives;

#[cfg(windows)]
impl DriveInspector for SystemDrives {
    fn kind(&self, letter: char) -> DriveKind {
        use std::os::windows::ffi::OsStrExt;
        let root: Vec<u16> = std::ffi::OsStr::new(&format!("{}:\\", letter.to_ascii_uppercase()))
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: `root` is a valid NUL-terminated UTF-16 string that outlives the call.
        let t = unsafe { windows_sys::Win32::Storage::FileSystem::GetDriveTypeW(root.as_ptr()) };
        match t {
            2 => DriveKind::Removable,
            3 => DriveKind::Fixed,
            4 => DriveKind::Network,
            5 => DriveKind::Optical,
            6 => DriveKind::Ram,
            _ => DriveKind::Unknown,
        }
    }
}

#[cfg(not(windows))]
impl DriveInspector for SystemDrives {
    fn kind(&self, _letter: char) -> DriveKind {
        DriveKind::Unknown
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Verdict {
    /// Reachable through the normal drive mount.
    Supported { wsl_path: String },
    /// Reachable, but with a caveat the user should know about.
    Warning { wsl_path: String, message: String },
    /// Cannot (or must not) be shared through a WSL mount.
    Unsupported { message: String },
}

impl Verdict {
    pub fn wsl_path(&self) -> Option<&str> {
        match self {
            Verdict::Supported { wsl_path } | Verdict::Warning { wsl_path, .. } => Some(wsl_path),
            Verdict::Unsupported { .. } => None,
        }
    }
}

pub const DEFAULT_MOUNT_ROOT: &str = "/mnt";

fn unsupported(msg: &str) -> Verdict {
    Verdict::Unsupported { message: msg.to_string() }
}

fn is_reserved_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or("").to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.chars().nth(3).is_some_and(|c| ('1'..='9').contains(&c)))
}

/// Translate an absolute Windows path to the path WSL would use, or explain why it cannot be
/// shared. Pure string logic (no filesystem access), so it behaves identically in tests.
///
/// `mount_root` is the distribution's automount root (`/mnt` by default; configurable per
/// distribution in `/etc/wsl.conf` — M4 verifies it with `wslpath` in a running distribution).
pub fn windows_to_wsl(path: &str, mount_root: &str, drives: &dyn DriveInspector) -> Verdict {
    let p = path.trim();
    if p.is_empty() {
        return unsupported("No folder was given.");
    }
    if p.chars().any(|c| c.is_control()) {
        return unsupported("The path contains control characters.");
    }
    if !mount_root.starts_with('/') {
        return unsupported("The distribution's mount root is not an absolute Linux path.");
    }
    let root = mount_root.trim_end_matches('/');

    let lower = p.to_ascii_lowercase();
    let mut rest: &str = p;
    if lower.starts_with("\\\\?\\unc\\") || lower.starts_with("//?/unc/") {
        return unsupported("This is a network (UNC) location. WSL cannot reach it through a normal drive mount; choose a folder on a local drive.");
    }
    if lower.starts_with("\\\\wsl$\\") || lower.starts_with("\\\\wsl.localhost\\") || lower.starts_with("//wsl$/") || lower.starts_with("//wsl.localhost/") {
        return unsupported("This location is inside a Linux distribution. Sharing it back into Linux would be circular.");
    }
    if lower.starts_with("\\\\.\\") {
        return unsupported("Device paths are not supported.");
    }
    if lower.starts_with("\\\\?\\") {
        rest = &p[4..];
    } else if lower.starts_with("\\\\") || lower.starts_with("//") {
        return unsupported("This is a network (UNC) location. WSL cannot reach it through a normal drive mount; choose a folder on a local drive.");
    }

    let b = rest.as_bytes();
    if b.len() < 2 || !b[0].is_ascii_alphabetic() || b[1] != b':' {
        return unsupported("Only absolute paths on a drive letter (for example D:\\Folder) are supported.");
    }
    if b.len() == 2 || (b[2] != b'\\' && b[2] != b'/') {
        return unsupported("The path is drive-relative; give the full path including the backslash after the drive letter.");
    }
    let letter = (b[0] as char).to_ascii_lowercase();

    let mut comps: Vec<&str> = Vec::new();
    for c in rest[3..].split(['\\', '/']) {
        if c.is_empty() || c == "." {
            continue;
        }
        if c == ".." {
            return unsupported("'..' is not allowed in a mapped path.");
        }
        if c.chars().any(|ch| matches!(ch, '<' | '>' | ':' | '"' | '|' | '?' | '*')) {
            return unsupported("The path contains characters Windows does not allow in names.");
        }
        if c.ends_with(' ') || c.ends_with('.') {
            return unsupported("A folder name ends with a space or a dot, which Windows does not preserve.");
        }
        if is_reserved_name(c) {
            return unsupported("The path contains a reserved Windows device name.");
        }
        comps.push(c);
    }
    if comps.is_empty() {
        return unsupported("A whole drive cannot be used as a mapping target; choose a folder on it.");
    }

    let wsl_path = format!("{root}/{letter}/{}", comps.join("/"));
    match drives.kind(letter) {
        DriveKind::Network => Verdict::Warning {
            wsl_path,
            message: format!("Drive {}: is a mapped network drive. WSL may not have it mounted, especially after sign-in or when the share is offline.", letter.to_ascii_uppercase()),
        },
        DriveKind::Removable | DriveKind::Optical => Verdict::Warning {
            wsl_path,
            message: format!("Drive {}: is removable media; the folder will be unavailable while it is disconnected.", letter.to_ascii_uppercase()),
        },
        _ => Verdict::Supported { wsl_path },
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FolderStatus {
    pub id: String,
    pub label: String,
    /// e.g. `~/Downloads`
    pub linux_path: String,
    pub mode: MappingMode,
    pub custom_windows_path: Option<String>,
    /// Where Windows currently keeps this folder (resolved now).
    pub default_windows_path: Option<String>,
    /// The Windows folder this mapping would use (custom, else default).
    pub effective_windows_path: Option<String>,
    /// `None` in private mode.
    pub verdict: Option<Verdict>,
    /// Plain-language answer to "where does this actually point?".
    pub points_to: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StorageOverview {
    pub folders: Vec<FolderStatus>,
    pub projects: ProjectsMode,
    pub mount_root: String,
    pub notes: Vec<String>,
}

pub fn overview(cfg: &StorageConfig, resolver: &dyn KnownFolderResolver, drives: &dyn DriveInspector, mount_root: &str) -> StorageOverview {
    let folders = KnownFolder::ALL
        .iter()
        .map(|&f| {
            let c = cfg.folder(f);
            let default = resolver.resolve(f);
            let effective = c.windows_path.clone().or_else(|| default.clone());
            let (verdict, points_to) = match c.mode {
                MappingMode::Private => (None, "Linux-private storage inside the distribution (not visible from Windows).".to_string()),
                MappingMode::Shared => match &effective {
                    None => {
                        let v = unsupported("Windows did not report a location for this folder.");
                        (Some(v), "Cannot be shared: Windows did not report a location for this folder.".to_string())
                    }
                    Some(path) => {
                        let v = windows_to_wsl(path, mount_root, drives);
                        let text = match &v {
                            Verdict::Supported { wsl_path } => format!("{path} (Windows) via {wsl_path}"),
                            Verdict::Warning { wsl_path, message } => format!("{path} (Windows) via {wsl_path} — {message}"),
                            Verdict::Unsupported { message } => format!("Cannot be shared: {message}"),
                        };
                        (Some(v), text)
                    }
                },
            };
            FolderStatus {
                id: f.id().to_string(),
                label: f.label().to_string(),
                linux_path: format!("~/{}", f.linux_dir()),
                mode: c.mode,
                custom_windows_path: c.windows_path.clone(),
                default_windows_path: default,
                effective_windows_path: effective,
                verdict,
                points_to,
            }
        })
        .collect();
    StorageOverview {
        folders,
        projects: cfg.projects,
        mount_root: mount_root.to_string(),
        notes: vec![
            "These are saved preferences. A folder is only linked to Windows after it has been checked and applied for a distribution; the status shown here comes from that check, never from the preference alone. Existing Linux folders are never replaced, merged or deleted without an explicit choice.".to_string(),
            "Linux-native storage is recommended for development projects: reading and writing files through /mnt is slower than the distribution's own filesystem.".to_string(),
            format!("Targets are shown with the default WSL mount root ({DEFAULT_MOUNT_ROOT}). When a mapping is checked or applied, the selected distribution's own /etc/wsl.conf is read, so a custom automount root is honoured."),
        ],
    }
}

/// What currently exists at `~/<Folder>` inside the distribution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinuxEntry {
    Missing,
    EmptyDir,
    NonEmptyDir,
    Symlink { target: String },
    File,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MappingAction {
    /// Already as desired.
    NoOp,
    /// Nothing exists there: create the link. No user data is involved.
    CreateLink { target: String },
    /// An empty directory exists. Replacing it loses nothing, but needs confirmation.
    ReplaceEmptyDir { target: String },
    /// A symlink points elsewhere. Only the link changes; needs confirmation.
    RetargetLink { from: String, target: String },
    /// Switching to private storage: only the symlink is removed and an empty private directory
    /// created. Files in the Windows folder are untouched. Needs confirmation.
    UnlinkToPrivate { from: String },
    /// Existing data is in the way (non-empty directory or a file). Never changed automatically;
    /// the user must decide (move/merge themselves, or keep this folder private).
    NeedsUserDecision { reason: String },
    /// The chosen Windows folder cannot be used.
    Blocked { reason: String },
}

impl MappingAction {
    pub fn requires_confirmation(&self) -> bool {
        matches!(self, MappingAction::ReplaceEmptyDir { .. } | MappingAction::RetargetLink { .. } | MappingAction::UnlinkToPrivate { .. })
    }
    /// Does this action delete or overwrite user files? Never (by construction).
    pub fn touches_user_files(&self) -> bool {
        false
    }
}

fn same_path(a: &str, b: &str) -> bool {
    a.trim_end_matches('/') == b.trim_end_matches('/')
}

pub fn plan_mapping(mode: MappingMode, verdict: Option<&Verdict>, linux: &LinuxEntry) -> MappingAction {
    match mode {
        MappingMode::Private => match linux {
            LinuxEntry::Symlink { target } => MappingAction::UnlinkToPrivate { from: target.clone() },
            _ => MappingAction::NoOp,
        },
        MappingMode::Shared => {
            let target = match verdict {
                Some(v) => match v.wsl_path() {
                    Some(p) => p.to_string(),
                    None => {
                        return MappingAction::Blocked {
                            reason: match v {
                                Verdict::Unsupported { message } => message.clone(),
                                _ => "unsupported".into(),
                            },
                        }
                    }
                },
                None => return MappingAction::Blocked { reason: "No Windows folder was chosen.".into() },
            };
            match linux {
                LinuxEntry::Missing => MappingAction::CreateLink { target },
                LinuxEntry::Symlink { target: cur } if same_path(cur, &target) => MappingAction::NoOp,
                LinuxEntry::Symlink { target: cur } => MappingAction::RetargetLink { from: cur.clone(), target },
                LinuxEntry::EmptyDir => MappingAction::ReplaceEmptyDir { target },
                LinuxEntry::NonEmptyDir => MappingAction::NeedsUserDecision {
                    reason: "The folder in Linux already contains files. VLINUX will not replace it. Move or merge its contents yourself, or keep this folder private.".into(),
                },
                LinuxEntry::File => MappingAction::NeedsUserDecision {
                    reason: "A file with this name exists in your Linux home. VLINUX will not overwrite it.".into(),
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Drives(HashMap<char, DriveKind>);
    impl DriveInspector for Drives {
        fn kind(&self, l: char) -> DriveKind {
            *self.0.get(&l).unwrap_or(&DriveKind::Fixed)
        }
    }
    fn fixed() -> Drives {
        Drives(HashMap::new())
    }

    struct Folders(HashMap<KnownFolder, String>);
    impl KnownFolderResolver for Folders {
        fn resolve(&self, f: KnownFolder) -> Option<String> {
            self.0.get(&f).cloned()
        }
    }

    fn sup(p: &str) -> Verdict {
        windows_to_wsl(p, "/mnt", &fixed())
    }

    #[test]
    fn ordinary_paths_map_to_mnt() {
        assert_eq!(sup(r"C:\Users\Someone\Downloads"), Verdict::Supported { wsl_path: "/mnt/c/Users/Someone/Downloads".into() });
        assert_eq!(sup(r"D:\Media\Pictures\"), Verdict::Supported { wsl_path: "/mnt/d/Media/Pictures".into() });
        assert_eq!(sup("e:/Docs/Work"), Verdict::Supported { wsl_path: "/mnt/e/Docs/Work".into() });
        assert_eq!(sup(r"\\?\C:\Users\X\Music"), Verdict::Supported { wsl_path: "/mnt/c/Users/X/Music".into() });
        assert_eq!(sup(r"C:\Users\Zoë\Dokumente"), Verdict::Supported { wsl_path: "/mnt/c/Users/Zoë/Dokumente".into() });
        assert_eq!(sup(r"C:\a\\b\.\c"), Verdict::Supported { wsl_path: "/mnt/c/a/b/c".into() });
    }

    #[test]
    fn custom_mount_root() {
        assert_eq!(windows_to_wsl(r"C:\x", "/windir/", &fixed()), Verdict::Supported { wsl_path: "/windir/c/x".into() });
        assert!(matches!(windows_to_wsl(r"C:\x", "mnt", &fixed()), Verdict::Unsupported { .. }));
    }

    #[test]
    fn unc_and_network_locations_are_detected_not_mapped() {
        for p in [r"\\server\share\Docs", r"\\?\UNC\server\share\x", "//server/share/x", r"\\wsl$\Ubuntu\home\me", r"\\wsl.localhost\Ubuntu\home", r"\\.\PHYSICALDRIVE0"] {
            assert!(matches!(sup(p), Verdict::Unsupported { .. }), "{p}");
        }
        match sup(r"\\wsl$\Ubuntu\home") {
            Verdict::Unsupported { message } => assert!(message.contains("circular")),
            v => panic!("{v:?}"),
        }
    }

    #[test]
    fn invalid_paths_are_rejected() {
        for p in ["", "   ", "Downloads", r"..\x", r"C:", r"C:Users\x", r"C:\", r"D:\\", r"C:\a\..\b", "C:\\a\u{0}b", r"C:\a<b", r"C:\a:stream", r"C:\folder.", r"C:\CON", r"C:\x\nul.txt", r"C:\lpt1", r"1:\x"] {
            assert!(matches!(sup(p), Verdict::Unsupported { .. }), "{p:?}");
        }
        assert!(matches!(sup(r"C:\COM10"), Verdict::Supported { .. }));
    }

    #[test]
    fn network_and_removable_drives_warn() {
        let d = Drives(HashMap::from([('z', DriveKind::Network), ('f', DriveKind::Removable)]));
        assert!(matches!(windows_to_wsl(r"Z:\Share\Docs", "/mnt", &d), Verdict::Warning { .. }));
        assert!(matches!(windows_to_wsl(r"F:\Usb", "/mnt", &d), Verdict::Warning { .. }));
        assert!(matches!(windows_to_wsl(r"C:\Fixed", "/mnt", &d), Verdict::Supported { .. }));
    }

    #[test]
    fn defaults_are_shared_with_dynamic_default_locations() {
        let cfg = StorageConfig::default();
        for f in KnownFolder::ALL {
            assert_eq!(cfg.folder(f).mode, MappingMode::Shared);
            assert_eq!(cfg.folder(f).windows_path, None);
        }
        assert_eq!(cfg.projects, ProjectsMode::LinuxPrivate);
    }

    #[test]
    fn overview_resolves_redirected_folders_and_other_drives() {
        // A user whose Documents is redirected to D: and whose Pictures is on a network share.
        let r = Folders(HashMap::from([
            (KnownFolder::Downloads, r"C:\Users\Anyone\Downloads".to_string()),
            (KnownFolder::Documents, r"D:\Redirected\Documents".to_string()),
            (KnownFolder::Pictures, r"\\nas\home\Pictures".to_string()),
        ]));
        let o = overview(&StorageConfig::default(), &r, &fixed(), "/mnt");
        let by = |id: &str| o.folders.iter().find(|f| f.id == id).unwrap().clone();
        assert_eq!(by("downloads").verdict, Some(Verdict::Supported { wsl_path: "/mnt/c/Users/Anyone/Downloads".into() }));
        assert_eq!(by("documents").verdict, Some(Verdict::Supported { wsl_path: "/mnt/d/Redirected/Documents".into() }));
        assert!(matches!(by("pictures").verdict, Some(Verdict::Unsupported { .. })));
        assert!(by("pictures").points_to.starts_with("Cannot be shared"));
        // No location reported at all.
        assert!(matches!(by("videos").verdict, Some(Verdict::Unsupported { .. })));
        assert_eq!(by("downloads").linux_path, "~/Downloads");
        assert_eq!(o.folders.len(), 6);
    }

    #[test]
    fn overview_custom_path_and_private_mode() {
        let r = Folders(HashMap::from([(KnownFolder::Downloads, r"C:\Users\A\Downloads".to_string())]));
        let mut cfg = StorageConfig::default();
        cfg.downloads.windows_path = Some(r"E:\Stuff\Dl".into());
        cfg.music.mode = MappingMode::Private;
        let o = overview(&cfg, &r, &fixed(), "/mnt");
        let dl = o.folders.iter().find(|f| f.id == "downloads").unwrap();
        assert_eq!(dl.effective_windows_path.as_deref(), Some(r"E:\Stuff\Dl"));
        assert_eq!(dl.default_windows_path.as_deref(), Some(r"C:\Users\A\Downloads"));
        assert_eq!(dl.verdict.as_ref().and_then(|v| v.wsl_path()), Some("/mnt/e/Stuff/Dl"));
        let music = o.folders.iter().find(|f| f.id == "music").unwrap();
        assert!(music.verdict.is_none());
        assert!(music.points_to.contains("Linux-private"));
    }

    #[test]
    fn config_sanitising() {
        let mut cfg = StorageConfig::default();
        cfg.downloads.windows_path = Some("   ".into());
        cfg.documents.windows_path = Some("  D:\\Docs  ".into());
        cfg.music.windows_path = Some("x\0y".into());
        cfg.pictures.windows_path = Some("C:\\".to_string() + &"a".repeat(2000));
        let s = cfg.sanitized();
        assert_eq!(s.downloads.windows_path, None);
        assert_eq!(s.documents.windows_path.as_deref(), Some("D:\\Docs"));
        assert_eq!(s.music.windows_path, None);
        assert_eq!(s.pictures.windows_path, None);
    }

    #[test]
    fn storage_config_serde_roundtrip_and_partial_files() {
        let mut cfg = StorageConfig::default();
        cfg.videos.mode = MappingMode::Private;
        cfg.desktop.windows_path = Some(r"D:\Desk".into());
        cfg.projects = ProjectsMode::Shared;
        let json = serde_json::to_string(&cfg).unwrap();
        assert_eq!(serde_json::from_str::<StorageConfig>(&json).unwrap(), cfg);
        let partial: StorageConfig = serde_json::from_str(r#"{"music":{"mode":"private"}}"#).unwrap();
        assert_eq!(partial.music.mode, MappingMode::Private);
        assert_eq!(partial.downloads.mode, MappingMode::Shared);
        assert!(serde_json::from_str::<StorageConfig>(r#"{"music":{"mode":"sideways"}}"#).is_err());
    }

    fn shared_ok() -> Verdict {
        Verdict::Supported { wsl_path: "/mnt/c/Users/A/Downloads".into() }
    }

    #[test]
    fn planner_never_touches_user_data() {
        let v = shared_ok();
        let t = "/mnt/c/Users/A/Downloads".to_string();
        assert_eq!(plan_mapping(MappingMode::Shared, Some(&v), &LinuxEntry::Missing), MappingAction::CreateLink { target: t.clone() });
        assert_eq!(plan_mapping(MappingMode::Shared, Some(&v), &LinuxEntry::Symlink { target: format!("{t}/") }), MappingAction::NoOp);
        assert!(matches!(plan_mapping(MappingMode::Shared, Some(&v), &LinuxEntry::Symlink { target: "/elsewhere".into() }), MappingAction::RetargetLink { .. }));
        assert!(matches!(plan_mapping(MappingMode::Shared, Some(&v), &LinuxEntry::EmptyDir), MappingAction::ReplaceEmptyDir { .. }));
        // Existing data: never replaced, handed back to the user.
        assert!(matches!(plan_mapping(MappingMode::Shared, Some(&v), &LinuxEntry::NonEmptyDir), MappingAction::NeedsUserDecision { .. }));
        assert!(matches!(plan_mapping(MappingMode::Shared, Some(&v), &LinuxEntry::File), MappingAction::NeedsUserDecision { .. }));
        // Unsupported Windows path.
        let bad = Verdict::Unsupported { message: "unc".into() };
        assert!(matches!(plan_mapping(MappingMode::Shared, Some(&bad), &LinuxEntry::Missing), MappingAction::Blocked { .. }));
        assert!(matches!(plan_mapping(MappingMode::Shared, None, &LinuxEntry::Missing), MappingAction::Blocked { .. }));
    }

    #[test]
    fn planner_private_mode_only_unlinks_symlinks() {
        for e in [LinuxEntry::Missing, LinuxEntry::EmptyDir, LinuxEntry::NonEmptyDir, LinuxEntry::File] {
            assert_eq!(plan_mapping(MappingMode::Private, None, &e), MappingAction::NoOp);
        }
        let a = plan_mapping(MappingMode::Private, None, &LinuxEntry::Symlink { target: "/mnt/c/x".into() });
        assert_eq!(a, MappingAction::UnlinkToPrivate { from: "/mnt/c/x".into() });
        assert!(a.requires_confirmation());
        assert!(!a.touches_user_files());
    }

    #[test]
    fn confirmation_rules() {
        let v = shared_ok();
        assert!(!plan_mapping(MappingMode::Shared, Some(&v), &LinuxEntry::Missing).requires_confirmation());
        assert!(plan_mapping(MappingMode::Shared, Some(&v), &LinuxEntry::EmptyDir).requires_confirmation());
        assert!(!plan_mapping(MappingMode::Shared, Some(&v), &LinuxEntry::NonEmptyDir).requires_confirmation());
    }
}
