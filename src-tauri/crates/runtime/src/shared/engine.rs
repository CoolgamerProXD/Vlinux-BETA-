//! The mapping engine: decides what state a `~/Downloads`-style folder is in, and applies or
//! undoes a link to the Windows folder — without ever overwriting, merging or deleting anything.
//!
//! Safety rules (each is covered by tests):
//! * Applying re-inspects the live filesystem; a stale "ready" status cannot cause damage.
//! * A non-empty folder or a file is never touched except by an explicit *move aside* (a rename
//!   to `<Name>.vlinux-backup`, never replacing anything), and that can be undone.
//! * An empty folder is removed with `rmdir` (which refuses a non-empty one). A foreign link is
//!   replaced only after an explicit choice, and the old target is recorded.
//! * Every step that can fail is rolled back, and the result is verified by reading the link
//!   back. If a mapping can't be applied safely, the filesystem is left as it was.
//! * Undo removes only a *link* (never a directory) that VLINUX can account for, then restores
//!   what was there before. Windows files are never touched.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::distro::{valid_exec_path, DistroOps};
use super::store::{MappingRecord, MappingStore, Replaced};
use crate::fs::path::linux_components;
use crate::storage::{plan_mapping, windows_to_wsl, DriveInspector, FolderConfig, KnownFolder, KnownFolderResolver, LinuxEntry, MappingAction, MappingMode, Verdict};

pub struct Ctx<'a> {
    pub distro: &'a str,
    /// The distribution's root as seen by the host (`\\wsl.localhost\<name>`).
    pub linux_root: &'a Path,
    pub home: &'a str,
    pub mount_root: &'a str,
    /// `[automount] enabled` from the distribution's `/etc/wsl.conf`.
    pub automount: bool,
    pub ops: &'a dyn DistroOps,
    pub drives: &'a dyn DriveInspector,
    pub resolver: &'a dyn KnownFolderResolver,
    /// Does this Windows folder exist right now?
    pub windows_dir_exists: &'a dyn Fn(&str) -> bool,
    pub now: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    /// Remove the *empty* Linux folder and link in its place.
    ReplaceEmptyFolder,
    /// Replace an existing symbolic link (the link only; its target is untouched).
    ReplaceLink,
    /// Rename the existing folder/file to `<Name>.vlinux-backup` and link in its place.
    MoveAside,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Offer {
    pub resolution: Resolution,
    pub label: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum MappingState {
    /// Preferred, but the live state has not been checked yet (no Linux process was started).
    Unknown,
    /// Linux-private storage: nothing links to Windows.
    Private,
    /// Safe to apply. `needs` is the confirmation that has to accompany Apply, if any.
    Ready { needs: Option<Resolution>, note: String },
    /// Verified live: the Linux folder is a link to the Windows folder.
    Applied { target: String },
    /// Something is in the way. Nothing was changed; the user has to choose.
    Conflict { kind: String, detail: String, entries: Vec<String>, more: usize, offers: Vec<Offer> },
    /// The preference is "private" but VLINUX's link is still in place. Undo removes it.
    LinkedButPrivate { target: String },
    /// Can't be applied right now (nothing was changed).
    Unavailable { reason: String },
    /// The live state couldn't be read.
    Failed { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MappingStatus {
    pub folder: String,
    pub label: String,
    pub linux_path: String,
    pub preference: MappingMode,
    pub windows_path: Option<String>,
    pub target: Option<String>,
    pub warning: Option<String>,
    pub state: MappingState,
    /// Where the original folder was moved to, if a backup exists for this mapping.
    pub backup: Option<String>,
    pub applied_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intended {
    pub windows_path: String,
    pub target: String,
    pub warning: Option<String>,
}

pub fn same_path(a: &str, b: &str) -> bool {
    a.trim_end_matches('/') == b.trim_end_matches('/')
}

pub fn link_path(ctx: &Ctx, folder: KnownFolder) -> String {
    format!("{}/{}", ctx.home.trim_end_matches('/'), folder.linux_dir())
}

fn host_of(ctx: &Ctx, linux: &str) -> PathBuf {
    let mut p = ctx.linux_root.to_path_buf();
    for c in linux_components(linux).unwrap_or_default() {
        p.push(c);
    }
    p
}

fn not_found(p: &Path) -> bool {
    matches!(fs::symlink_metadata(p), Err(e) if e.kind() == io::ErrorKind::NotFound)
}

/// Where the Windows folder is and how Linux reaches it — or why it can't.
pub fn intended(ctx: &Ctx, folder: KnownFolder, cfg: &FolderConfig) -> Result<Intended, String> {
    if !ctx.automount {
        return Err("Windows drives are not mounted in this distribution (automount is disabled in /etc/wsl.conf), so a Windows folder can't be linked.".into());
    }
    let windows_path = cfg
        .windows_path
        .clone()
        .or_else(|| ctx.resolver.resolve(folder))
        .ok_or_else(|| "Windows did not report a location for this folder.".to_string())?;
    let (target, warning) = match windows_to_wsl(&windows_path, ctx.mount_root, ctx.drives) {
        Verdict::Supported { wsl_path } => (wsl_path, None),
        Verdict::Warning { wsl_path, message } => (wsl_path, Some(message)),
        Verdict::Unsupported { message } => return Err(message),
    };
    if !valid_exec_path(&target) {
        return Err("The path contains characters that can't be linked safely.".into());
    }
    if !(ctx.windows_dir_exists)(&windows_path) {
        return Err(format!("The Windows folder {windows_path} does not exist or can't be reached right now."));
    }
    Ok(Intended { windows_path, target, warning })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inspected {
    pub entry: LinuxEntry,
    /// Up to 8 names inside a non-empty folder, for the conflict explanation.
    pub names: Vec<String>,
    pub more: usize,
}

/// What exists at `~/<Folder>` right now. Errors are reported, never guessed around.
pub fn inspect(ctx: &Ctx, folder: KnownFolder) -> Result<Inspected, String> {
    let link = link_path(ctx, folder);
    let host = host_of(ctx, &link);
    let plain = |entry| Inspected { entry, names: Vec::new(), more: 0 };
    // The distribution is the authority on links (the host file share may follow or hide them).
    match ctx.ops.read_link(&link) {
        Ok(Some(target)) => return Ok(plain(LinuxEntry::Symlink { target })),
        Ok(None) => {}
        Err(e) => return Err(format!("Couldn't inspect {link}: {e}")),
    }
    let meta = match fs::symlink_metadata(&host) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(plain(LinuxEntry::Missing)),
        Err(e) => return Err(format!("Couldn't inspect {link}: {e}")),
    };
    if meta.is_dir() {
        let mut names = Vec::new();
        let mut count = 0usize;
        match fs::read_dir(&host) {
            Ok(it) => {
                for item in it {
                    match item {
                        Ok(d) => {
                            count += 1;
                            if names.len() < 8 {
                                names.push(d.file_name().to_string_lossy().into_owned());
                            }
                            if count >= 1000 {
                                break;
                            }
                        }
                        Err(e) => return Err(format!("Couldn't read {link}: {e}")),
                    }
                }
            }
            Err(e) => return Err(format!("Couldn't read {link}: {e}")),
        }
        names.sort();
        if count == 0 {
            Ok(plain(LinuxEntry::EmptyDir))
        } else {
            Ok(Inspected { entry: LinuxEntry::NonEmptyDir, names, more: count.saturating_sub(8) })
        }
    } else {
        Ok(plain(LinuxEntry::File))
    }
}

fn offer_move_aside(folder: KnownFolder) -> Offer {
    Offer {
        resolution: Resolution::MoveAside,
        label: "Move my existing items aside".into(),
        description: format!(
            "Renames ~/{0} to ~/{0}.vlinux-backup (nothing is copied or deleted), then links ~/{0} to the Windows folder. Undo moves it back.",
            folder.linux_dir()
        ),
    }
}

fn recorded_target_matches(rec: &Option<MappingRecord>, target: &str) -> bool {
    rec.as_ref().is_some_and(|r| same_path(&r.target, target))
}

pub fn status(ctx: &Ctx, store: &MappingStore, folder: KnownFolder, cfg: &FolderConfig, inspect_now: bool) -> MappingStatus {
    let rec = store.find(ctx.distro, folder.id());
    let want = intended(ctx, folder, cfg);
    let (windows_path, target, warning) = match &want {
        Ok(i) => (Some(i.windows_path.clone()), Some(i.target.clone()), i.warning.clone()),
        Err(_) => (cfg.windows_path.clone().or_else(|| ctx.resolver.resolve(folder)), None, None),
    };
    let backup = rec.as_ref().and_then(|r| match &r.replaced {
        Replaced::MovedAside { backup } => Some(backup.clone()),
        _ => None,
    });
    let make = |state: MappingState| MappingStatus {
        folder: folder.id().to_string(),
        label: folder.label().to_string(),
        linux_path: format!("~/{}", folder.linux_dir()),
        preference: cfg.mode,
        windows_path: windows_path.clone(),
        target: target.clone(),
        warning: warning.clone(),
        state,
        backup: backup.clone(),
        applied_at: rec.as_ref().map(|r| r.applied_at),
    };
    if !inspect_now {
        return make(if cfg.mode == MappingMode::Private { MappingState::Private } else { MappingState::Unknown });
    }
    let found = match inspect(ctx, folder) {
        Ok(f) => f,
        Err(message) => return make(MappingState::Failed { message }),
    };
    if cfg.mode == MappingMode::Private {
        return make(match &found.entry {
            LinuxEntry::Symlink { target } if recorded_target_matches(&rec, target) => MappingState::LinkedButPrivate { target: target.clone() },
            _ => MappingState::Private,
        });
    }
    let want = match want {
        Ok(w) => w,
        Err(reason) => return make(MappingState::Unavailable { reason }),
    };
    let verdict = Verdict::Supported { wsl_path: want.target.clone() };
    let link = format!("~/{}", folder.linux_dir());
    match plan_mapping(MappingMode::Shared, Some(&verdict), &found.entry) {
        MappingAction::NoOp => make(MappingState::Applied { target: want.target }),
        MappingAction::CreateLink { .. } => make(MappingState::Ready { needs: None, note: format!("{link} doesn't exist yet; a link will be created.") }),
        MappingAction::ReplaceEmptyDir { .. } => make(MappingState::Ready {
            needs: Some(Resolution::ReplaceEmptyFolder),
            note: format!("{link} is an empty folder. It will be replaced by the link (nothing is lost)."),
        }),
        MappingAction::RetargetLink { from, .. } => {
            if recorded_target_matches(&rec, &from) {
                make(MappingState::Ready {
                    needs: Some(Resolution::ReplaceLink),
                    note: format!("{link} currently points to {from}. It will be re-pointed to {}.", want.target),
                })
            } else {
                make(MappingState::Conflict {
                    kind: "other_link".into(),
                    detail: format!("{link} is already a symbolic link to {from}, which VLINUX didn't create."),
                    entries: Vec::new(),
                    more: 0,
                    offers: vec![Offer {
                        resolution: Resolution::ReplaceLink,
                        label: "Replace the link".into(),
                        description: format!("Re-points ~/{} to the Windows folder. Only the link changes; {from} is untouched, and Undo restores the old link.", folder.linux_dir()),
                    }],
                })
            }
        }
        MappingAction::NeedsUserDecision { reason } => {
            let (kind, detail) = if matches!(found.entry, LinuxEntry::File) { ("file", reason) } else { ("non_empty_folder", reason) };
            make(MappingState::Conflict { kind: kind.into(), detail, entries: found.names, more: found.more, offers: vec![offer_move_aside(folder)] })
        }
        MappingAction::Blocked { reason } => make(MappingState::Unavailable { reason }),
        MappingAction::UnlinkToPrivate { .. } => make(MappingState::Private),
    }
}

fn backup_path(ctx: &Ctx, folder: KnownFolder) -> Result<String, String> {
    let base = format!("{}.vlinux-backup", link_path(ctx, folder));
    for n in 1..100 {
        let cand = if n == 1 { base.clone() } else { format!("{base}-{n}") };
        if not_found(&host_of(ctx, &cand)) {
            return Ok(cand);
        }
    }
    Err("There are too many existing backups of this folder. Nothing was changed.".into())
}

fn create_verified(ctx: &Ctx, target: &str, link: &str) -> Result<(), String> {
    ctx.ops.make_symlink(target, link).map_err(|e| e.to_string())?;
    match ctx.ops.read_link(link) {
        Ok(Some(t)) if same_path(&t, target) => Ok(()),
        other => {
            // Something is off: take back only the link we just made.
            let _ = ctx.ops.remove_symlink(link);
            Err(format!("The link was created but could not be verified ({}).", match other {
                Ok(Some(t)) => format!("it points to {t}"),
                Ok(None) => "it is not a link".to_string(),
                Err(e) => e.to_string(),
            }))
        }
    }
}

fn record(ctx: &Ctx, folder: KnownFolder, want: &Intended, replaced: Replaced) -> MappingRecord {
    MappingRecord {
        distro: ctx.distro.to_string(),
        folder: folder.id().to_string(),
        linux_path: link_path(ctx, folder),
        target: want.target.clone(),
        windows_path: want.windows_path.clone(),
        applied_at: ctx.now,
        replaced,
    }
}

fn save(store: &MappingStore, ctx: &Ctx, rec: MappingRecord) -> Result<(), String> {
    store.upsert(rec, ctx.now).map_err(|e| format!("The link is in place, but VLINUX couldn't save its record ({e}); Undo will still recognise the link."))
}

fn need(resolution: Option<Resolution>, wanted: Resolution, what: &str) -> Result<(), String> {
    if resolution == Some(wanted) {
        Ok(())
    } else {
        Err(format!("{what} needs your confirmation first. Nothing was changed."))
    }
}

/// Apply the preference for one folder. `resolution` is the user's explicit choice, if the
/// status asked for one. Returns a plain-language summary.
pub fn apply(ctx: &Ctx, store: &MappingStore, folder: KnownFolder, cfg: &FolderConfig, resolution: Option<Resolution>) -> Result<String, String> {
    if cfg.mode != MappingMode::Shared {
        return Err("This folder is set to Linux-private storage, so there is nothing to link.".into());
    }
    let want = intended(ctx, folder, cfg).map_err(|e| format!("{e} Nothing was changed."))?;
    let link = link_path(ctx, folder);
    let shown = format!("~/{}", folder.linux_dir());
    let found = inspect(ctx, folder).map_err(|e| format!("{e} Nothing was changed."))?;
    let action = plan_mapping(MappingMode::Shared, Some(&Verdict::Supported { wsl_path: want.target.clone() }), &found.entry);
    if action != MappingAction::NoOp {
        match ctx.ops.dir_exists(&want.target) {
            Ok(true) => {}
            Ok(false) => {
                return Err(format!(
                    "{} is not reachable from inside {} (the drive may not be mounted there). Nothing was changed.",
                    want.target, ctx.distro
                ))
            }
            Err(e) => return Err(format!("Couldn't check {} inside {}: {e}. Nothing was changed.", want.target, ctx.distro)),
        }
    }
    let prior = store.find(ctx.distro, folder.id());
    match action {
        MappingAction::NoOp => {
            if prior.is_none() {
                save(store, ctx, record(ctx, folder, &want, Replaced::Adopted))?;
            }
            Ok(format!("{shown} was already linked to {}. Nothing was changed.", want.windows_path))
        }
        MappingAction::CreateLink { target } => {
            create_verified(ctx, &target, &link).map_err(|e| format!("{e} Nothing was changed."))?;
            save(store, ctx, record(ctx, folder, &want, Replaced::Nothing))?;
            Ok(format!("{shown} now opens {} directly.", want.windows_path))
        }
        MappingAction::ReplaceEmptyDir { target } => {
            need(resolution, Resolution::ReplaceEmptyFolder, &format!("Replacing the empty folder {shown}"))?;
            ctx.ops.remove_empty_dir(&link).map_err(|e| format!("{e}. Nothing was changed."))?;
            if let Err(e) = create_verified(ctx, &target, &link) {
                let restored = ctx.ops.make_dir(&link).is_ok();
                return Err(format!("{e} {}", if restored { "The empty folder was put back." } else { "Couldn't put the empty folder back; please recreate it." }));
            }
            save(store, ctx, record(ctx, folder, &want, Replaced::EmptyDir))?;
            Ok(format!("{shown} now opens {} directly.", want.windows_path))
        }
        MappingAction::RetargetLink { from, target } => {
            need(resolution, Resolution::ReplaceLink, &format!("Re-pointing the link {shown}"))?;
            let ours = recorded_target_matches(&prior, &from);
            ctx.ops.remove_symlink(&link).map_err(|e| format!("{e}. Nothing was changed."))?;
            if let Err(e) = create_verified(ctx, &target, &link) {
                let restored = ctx.ops.make_symlink(&from, &link).is_ok();
                return Err(if restored {
                    format!("{e} The previous link was restored.")
                } else {
                    format!("{e} Couldn't restore the previous link, which pointed to {from}.")
                });
            }
            let replaced = match (&prior, ours) {
                (Some(r), true) => r.replaced.clone(),
                _ => Replaced::Link { old_target: from },
            };
            save(store, ctx, record(ctx, folder, &want, replaced))?;
            Ok(format!("{shown} now opens {} directly.", want.windows_path))
        }
        MappingAction::NeedsUserDecision { reason } => {
            if resolution != Some(Resolution::MoveAside) {
                return Err(format!("{reason} Nothing was changed."));
            }
            let backup = backup_path(ctx, folder)?;
            ctx.ops.rename_noclobber(&link, &backup).map_err(|e| format!("{e}. Nothing was changed."))?;
            if not_found(&host_of(ctx, &backup)) {
                let _ = ctx.ops.rename_noclobber(&backup, &link);
                return Err("The existing items couldn't be verified at the backup location, so the move was undone where possible. Please check the folder.".into());
            }
            if let Err(e) = create_verified(ctx, &want.target, &link) {
                return Err(match ctx.ops.rename_noclobber(&backup, &link) {
                    Ok(()) => format!("{e} Your original {shown} was moved back."),
                    Err(back) => format!("{e} Your original items are safe at {backup}, but moving them back failed: {back}"),
                });
            }
            save(store, ctx, record(ctx, folder, &want, Replaced::MovedAside { backup: backup.clone() }))?;
            Ok(format!("{shown} now opens {}. Your previous contents are kept at {backup}; nothing was copied or deleted.", want.windows_path))
        }
        MappingAction::Blocked { reason } => Err(format!("{reason} Nothing was changed.")),
        MappingAction::UnlinkToPrivate { .. } => Err("This folder is set to Linux-private storage, so there is nothing to link.".into()),
    }
}

fn restore(ctx: &Ctx, link: &str, shown: &str, replaced: &Replaced) -> Result<String, String> {
    match replaced {
        Replaced::MovedAside { backup } => {
            if not_found(&host_of(ctx, backup)) {
                ctx.ops.make_dir(link).map_err(|e| format!("The link was removed, but {shown} couldn't be recreated: {e}"))?;
                Ok(format!("The link was removed. The backup {backup} no longer exists, so {shown} is a new empty folder."))
            } else {
                ctx.ops
                    .rename_noclobber(backup, link)
                    .map_err(|e| format!("The link was removed, but your original items couldn't be moved back: {e}. They are still at {backup}."))?;
                Ok(format!("The link was removed and your original {shown} was moved back."))
            }
        }
        Replaced::Link { old_target } => {
            ctx.ops.make_symlink(old_target, link).map_err(|e| format!("The link was removed, but the previous link to {old_target} couldn't be restored: {e}"))?;
            Ok(format!("The link was removed and {shown} points to {old_target} again."))
        }
        Replaced::Nothing | Replaced::EmptyDir | Replaced::Adopted => {
            ctx.ops.make_dir(link).map_err(|e| format!("The link was removed, but {shown} couldn't be recreated: {e}"))?;
            Ok(format!("The link was removed. {shown} is now a private folder inside the distribution. Files in the Windows folder were not touched."))
        }
    }
}

/// Undo: remove the link VLINUX made and put back what was there. Never deletes data.
pub fn undo(ctx: &Ctx, store: &MappingStore, folder: KnownFolder, cfg: &FolderConfig) -> Result<String, String> {
    let link = link_path(ctx, folder);
    let shown = format!("~/{}", folder.linux_dir());
    let rec = store.find(ctx.distro, folder.id());
    let found = inspect(ctx, folder).map_err(|e| format!("{e} Nothing was changed."))?;
    let replaced = match found.entry {
        LinuxEntry::Symlink { target } => {
            let mut expected: Vec<String> = Vec::new();
            if let Some(r) = &rec {
                expected.push(r.target.clone());
            }
            if let Ok(i) = intended(ctx, folder, cfg) {
                expected.push(i.target);
            }
            if !expected.iter().any(|t| same_path(t, &target)) {
                return Err(format!("{shown} is a link to {target}, which VLINUX didn't create, so it was left alone."));
            }
            ctx.ops.remove_symlink(&link).map_err(|e| format!("{e}. Nothing was changed."))?;
            rec.as_ref().map(|r| r.replaced.clone()).unwrap_or(Replaced::Adopted)
        }
        LinuxEntry::Missing => match &rec {
            Some(r) => r.replaced.clone(),
            None => return Ok(format!("{shown} isn't linked, so there was nothing to undo.")),
        },
        _ => {
            return Err(format!("{shown} is a regular folder or file, not a link VLINUX created. Nothing was changed."));
        }
    };
    let message = restore(ctx, &link, &shown, &replaced)?;
    store.remove(ctx.distro, folder.id()).map_err(|e| format!("{message} (VLINUX couldn't update its record: {e})"))?;
    Ok(message)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::fs::testutil::TempDir;
    use crate::shared::distro::OpsError;
    use crate::shared::testing::FakeDistro;
    use crate::storage::DriveKind;
    use std::collections::HashMap;

    struct Folders(HashMap<KnownFolder, String>);
    impl KnownFolderResolver for Folders {
        fn resolve(&self, f: KnownFolder) -> Option<String> {
            self.0.get(&f).cloned()
        }
    }
    struct Fixed;
    impl DriveInspector for Fixed {
        fn kind(&self, _l: char) -> DriveKind {
            DriveKind::Fixed
        }
    }

    const WIN_DL: &str = "D:\\Users\\Someone\\Downloads";

    /// A fake distro rooted in a temp dir, with Windows drive D: mounted at /mnt/d.
    struct Env {
        _t: TempDir,
        root: PathBuf,
        win: PathBuf,
        cfg: TempDir,
        distro: FakeDistro,
        folders: Folders,
        shared: FolderConfig,
        private: FolderConfig,
    }

    impl Env {
        fn new() -> Self {
            let t = TempDir::new("engine");
            let root = t.dir("distro");
            t.dir("distro/home/me");
            let win = t.dir("windows-d");
            t.dir("windows-d/Users/Someone/Downloads");
            t.dir("windows-d/Users/Someone/Documents");
            let distro = FakeDistro::new(&root, "/home/me").mount("/mnt/d", &win);
            let mut f = HashMap::new();
            f.insert(KnownFolder::Downloads, WIN_DL.to_string());
            f.insert(KnownFolder::Documents, "D:\\Users\\Someone\\Documents".to_string());
            Env {
                _t: t,
                root,
                win,
                cfg: TempDir::new("engine-cfg"),
                distro,
                folders: Folders(f),
                shared: FolderConfig::default(),
                private: FolderConfig { mode: MappingMode::Private, windows_path: None },
            }
        }
        fn store(&self) -> MappingStore {
            MappingStore::new(self.cfg.path())
        }
        fn home(&self, rel: &str) -> PathBuf {
            self.root.join("home").join("me").join(rel)
        }
        fn with<T>(&self, f: impl FnOnce(&Ctx) -> T) -> T {
            let win = self.win.clone();
            let exists = move |p: &str| {
                // Only drive D: exists in this fake machine.
                match p.strip_prefix("D:\\") {
                    Some(rest) => {
                        let mut h = win.clone();
                        for c in rest.split('\\').filter(|c| !c.is_empty()) {
                            h.push(c);
                        }
                        h.is_dir()
                    }
                    None => false,
                }
            };
            let ctx = Ctx {
                distro: "Debian",
                linux_root: &self.root,
                home: "/home/me",
                mount_root: "/mnt",
                automount: true,
                ops: &self.distro,
                drives: &Fixed,
                resolver: &self.folders,
                windows_dir_exists: &exists,
                now: 1000,
            };
            f(&ctx)
        }
        fn status(&self, cfg: &FolderConfig, inspect: bool) -> MappingState {
            let store = self.store();
            self.with(|c| status(c, &store, KnownFolder::Downloads, cfg, inspect)).state
        }
        fn apply(&self, cfg: &FolderConfig, r: Option<Resolution>) -> Result<String, String> {
            let store = self.store();
            self.with(|c| apply(c, &store, KnownFolder::Downloads, cfg, r))
        }
        fn undo(&self, cfg: &FolderConfig) -> Result<String, String> {
            let store = self.store();
            self.with(|c| undo(c, &store, KnownFolder::Downloads, cfg))
        }
        fn is_link(&self, rel: &str) -> bool {
            fs::symlink_metadata(self.home(rel)).map(|m| m.file_type().is_symlink()).unwrap_or(false)
        }
    }

    #[test]
    fn unchecked_status_does_not_touch_anything_or_claim_connection() {
        let e = Env::new();
        assert_eq!(e.status(&e.shared, false), MappingState::Unknown);
        assert_eq!(e.status(&e.private, false), MappingState::Private);
        assert!(e.distro.calls().is_empty(), "no distro process for an unchecked status");
    }

    #[test]
    fn missing_folder_is_ready_and_apply_links_it_with_immediate_two_way_visibility() {
        let e = Env::new();
        assert!(matches!(e.status(&e.shared, true), MappingState::Ready { needs: None, .. }));
        let msg = e.apply(&e.shared, None).unwrap();
        assert!(msg.contains(WIN_DL), "{msg}");
        assert!(e.is_link("Downloads"));
        assert_eq!(e.status(&e.shared, true), MappingState::Applied { target: "/mnt/d/Users/Someone/Downloads".into() });
        // Linux -> Windows: a file created through ~/Downloads is in the Windows folder at once.
        fs::write(e.home("Downloads/from-linux.txt"), "hi").unwrap();
        assert_eq!(fs::read_to_string(e.win.join("Users/Someone/Downloads/from-linux.txt")).unwrap(), "hi");
        // Windows -> Linux.
        fs::write(e.win.join("Users/Someone/Downloads/from-windows.txt"), "yo").unwrap();
        assert_eq!(fs::read_to_string(e.home("Downloads/from-windows.txt")).unwrap(), "yo");
        // Recorded for reversal.
        let rec = e.store().find("Debian", "downloads").unwrap();
        assert_eq!(rec.replaced, Replaced::Nothing);
        assert_eq!(rec.windows_path, WIN_DL);
    }

    #[test]
    fn applying_twice_is_a_no_op() {
        let e = Env::new();
        e.apply(&e.shared, None).unwrap();
        let calls = e.distro.calls().iter().filter(|c| c.starts_with("make_symlink")).count();
        let msg = e.apply(&e.shared, None).unwrap();
        assert!(msg.contains("already linked"), "{msg}");
        assert_eq!(e.distro.calls().iter().filter(|c| c.starts_with("make_symlink")).count(), calls);
    }

    #[test]
    fn empty_folder_needs_confirmation_and_is_restored_by_undo() {
        let e = Env::new();
        fs::create_dir(e.home("Downloads")).unwrap();
        assert_eq!(
            e.status(&e.shared, true),
            MappingState::Ready { needs: Some(Resolution::ReplaceEmptyFolder), note: "~/Downloads is an empty folder. It will be replaced by the link (nothing is lost).".into() }
        );
        // No confirmation: refused, nothing changed.
        let err = e.apply(&e.shared, None).unwrap_err();
        assert!(err.contains("confirmation"), "{err}");
        assert!(!e.is_link("Downloads") && e.home("Downloads").is_dir());
        // The wrong confirmation is not enough either.
        assert!(e.apply(&e.shared, Some(Resolution::MoveAside)).is_err());
        e.apply(&e.shared, Some(Resolution::ReplaceEmptyFolder)).unwrap();
        assert!(e.is_link("Downloads"));
        assert_eq!(e.store().find("Debian", "downloads").unwrap().replaced, Replaced::EmptyDir);
        e.undo(&e.shared).unwrap();
        assert!(!e.is_link("Downloads") && e.home("Downloads").is_dir());
        assert!(e.store().find("Debian", "downloads").is_none());
    }

    #[test]
    fn a_non_empty_folder_is_a_conflict_and_is_never_touched_without_an_explicit_choice() {
        let e = Env::new();
        fs::create_dir(e.home("Downloads")).unwrap();
        fs::write(e.home("Downloads/thesis.odt"), "precious").unwrap();
        fs::create_dir(e.home("Downloads/sub")).unwrap();
        match e.status(&e.shared, true) {
            MappingState::Conflict { kind, entries, offers, detail, .. } => {
                assert_eq!(kind, "non_empty_folder");
                assert_eq!(entries, ["sub", "thesis.odt"]);
                assert_eq!(offers.len(), 1);
                assert_eq!(offers[0].resolution, Resolution::MoveAside);
                assert!(detail.contains("will not replace"), "{detail}");
            }
            other => panic!("unexpected {other:?}"),
        }
        for r in [None, Some(Resolution::ReplaceEmptyFolder), Some(Resolution::ReplaceLink)] {
            assert!(e.apply(&e.shared, r).is_err());
        }
        assert_eq!(fs::read_to_string(e.home("Downloads/thesis.odt")).unwrap(), "precious");
        assert!(!e.is_link("Downloads"));
        assert!(e.store().all().is_empty());
        assert!(!e.distro.calls().iter().any(|c| c.starts_with("make_symlink") || c.starts_with("remove") || c.starts_with("rename")));
    }

    #[test]
    fn move_aside_keeps_everything_and_undo_moves_it_back() {
        let e = Env::new();
        fs::create_dir(e.home("Downloads")).unwrap();
        fs::write(e.home("Downloads/thesis.odt"), "precious").unwrap();
        let msg = e.apply(&e.shared, Some(Resolution::MoveAside)).unwrap();
        assert!(msg.contains("Downloads.vlinux-backup"), "{msg}");
        assert!(e.is_link("Downloads"));
        assert_eq!(fs::read_to_string(e.home("Downloads.vlinux-backup/thesis.odt")).unwrap(), "precious");
        // Windows files are not touched and the user's file was not copied there.
        assert!(!e.win.join("Users/Someone/Downloads/thesis.odt").exists());
        match e.status(&e.shared, true) {
            MappingState::Applied { .. } => {}
            other => panic!("{other:?}"),
        }
        let st = e.store().find("Debian", "downloads").unwrap();
        assert_eq!(st.replaced, Replaced::MovedAside { backup: "/home/me/Downloads.vlinux-backup".into() });
        // Unrelated Windows file survives the undo.
        fs::write(e.win.join("Users/Someone/Downloads/keep.txt"), "k").unwrap();
        e.undo(&e.shared).unwrap();
        assert!(!e.is_link("Downloads"));
        assert_eq!(fs::read_to_string(e.home("Downloads/thesis.odt")).unwrap(), "precious");
        assert!(!e.home("Downloads.vlinux-backup").exists());
        assert_eq!(fs::read_to_string(e.win.join("Users/Someone/Downloads/keep.txt")).unwrap(), "k");
    }

    #[test]
    fn backup_names_never_collide() {
        let e = Env::new();
        fs::create_dir(e.home("Downloads.vlinux-backup")).unwrap();
        fs::write(e.home("Downloads.vlinux-backup/old.txt"), "old").unwrap();
        fs::create_dir(e.home("Downloads")).unwrap();
        fs::write(e.home("Downloads/new.txt"), "new").unwrap();
        e.apply(&e.shared, Some(Resolution::MoveAside)).unwrap();
        assert_eq!(fs::read_to_string(e.home("Downloads.vlinux-backup/old.txt")).unwrap(), "old");
        assert_eq!(fs::read_to_string(e.home("Downloads.vlinux-backup-2/new.txt")).unwrap(), "new");
    }

    #[test]
    fn a_regular_file_in_the_way_is_a_conflict_too() {
        let e = Env::new();
        fs::write(e.home("Downloads"), "i am a file").unwrap();
        match e.status(&e.shared, true) {
            MappingState::Conflict { kind, .. } => assert_eq!(kind, "file"),
            other => panic!("{other:?}"),
        }
        assert!(e.apply(&e.shared, None).is_err());
        assert_eq!(fs::read_to_string(e.home("Downloads")).unwrap(), "i am a file");
        e.apply(&e.shared, Some(Resolution::MoveAside)).unwrap();
        assert_eq!(fs::read_to_string(e.home("Downloads.vlinux-backup")).unwrap(), "i am a file");
        e.undo(&e.shared).unwrap();
        assert_eq!(fs::read_to_string(e.home("Downloads")).unwrap(), "i am a file");
    }

    #[test]
    fn a_foreign_symlink_is_never_silently_replaced() {
        let e = Env::new();
        fs::create_dir(e.root.join("elsewhere")).unwrap();
        std::os::unix::fs::symlink(e.root.join("elsewhere"), e.home("Downloads")).unwrap();
        match e.status(&e.shared, true) {
            MappingState::Conflict { kind, detail, offers, .. } => {
                assert_eq!(kind, "other_link");
                assert!(detail.contains("/elsewhere"), "{detail}");
                assert_eq!(offers[0].resolution, Resolution::ReplaceLink);
            }
            other => panic!("{other:?}"),
        }
        assert!(e.apply(&e.shared, None).is_err());
        assert_eq!(fs::read_link(e.home("Downloads")).unwrap(), e.root.join("elsewhere"));
        // Undo must not remove a link it can't account for.
        let err = e.undo(&e.shared).unwrap_err();
        assert!(err.contains("didn't create"), "{err}");
        assert!(e.is_link("Downloads"));
        // With the explicit choice the link (only) is replaced, and undo restores the old one.
        e.apply(&e.shared, Some(Resolution::ReplaceLink)).unwrap();
        assert!(e.root.join("elsewhere").is_dir());
        assert_eq!(fs::read_link(e.home("Downloads")).unwrap(), e.win.join("Users/Someone/Downloads"));
        e.undo(&e.shared).unwrap();
        assert_eq!(fs::read_link(e.home("Downloads")).unwrap(), e.root.join("elsewhere"));
    }

    #[test]
    fn changing_the_windows_destination_repoints_our_own_link_and_keeps_the_original_restore() {
        let e = Env::new();
        fs::create_dir(e.home("Downloads")).unwrap();
        fs::write(e.home("Downloads/mine.txt"), "m").unwrap();
        e.apply(&e.shared, Some(Resolution::MoveAside)).unwrap();
        fs::create_dir_all(e.win.join("Other/Dl")).unwrap();
        let custom = FolderConfig { mode: MappingMode::Shared, windows_path: Some("D:\\Other\\Dl".into()) };
        match e.status(&custom, true) {
            MappingState::Ready { needs: Some(Resolution::ReplaceLink), .. } => {}
            other => panic!("{other:?}"),
        }
        assert!(e.apply(&custom, None).is_err());
        e.apply(&custom, Some(Resolution::ReplaceLink)).unwrap();
        assert_eq!(fs::read_link(e.home("Downloads")).unwrap(), e.win.join("Other/Dl"));
        // The very first original is still what undo restores.
        e.undo(&custom).unwrap();
        assert_eq!(fs::read_to_string(e.home("Downloads/mine.txt")).unwrap(), "m");
    }

    #[test]
    fn disabling_a_mapping_keeps_all_data_on_both_sides() {
        let e = Env::new();
        e.apply(&e.shared, None).unwrap();
        fs::write(e.home("Downloads/report.pdf"), "pdf").unwrap();
        assert!(matches!(e.status(&e.private, true), MappingState::LinkedButPrivate { .. }));
        e.undo(&e.private).unwrap();
        assert!(!e.is_link("Downloads") && e.home("Downloads").is_dir());
        assert_eq!(fs::read_dir(e.home("Downloads")).unwrap().count(), 0, "private folder starts empty");
        assert_eq!(fs::read_to_string(e.win.join("Users/Someone/Downloads/report.pdf")).unwrap(), "pdf");
        assert_eq!(e.status(&e.private, true), MappingState::Private);
        // Re-enabling works again.
        e.apply(&e.shared, Some(Resolution::ReplaceEmptyFolder)).unwrap();
        assert!(e.is_link("Downloads"));
    }

    #[test]
    fn undo_of_something_that_is_not_ours_changes_nothing() {
        let e = Env::new();
        fs::create_dir(e.home("Downloads")).unwrap();
        fs::write(e.home("Downloads/x"), "x").unwrap();
        let err = e.undo(&e.shared).unwrap_err();
        assert!(err.contains("not a link"), "{err}");
        assert!(e.home("Downloads/x").exists());
        // Nothing there and no record: nothing to do.
        let e2 = Env::new();
        assert!(e2.undo(&e2.shared).unwrap().contains("nothing to undo"));
    }

    #[test]
    fn unavailable_destinations_leave_the_filesystem_unchanged() {
        let e = Env::new();
        fs::create_dir(e.home("Downloads")).unwrap();
        // UNC / network path.
        let unc = FolderConfig { mode: MappingMode::Shared, windows_path: Some("\\\\nas\\share\\Downloads".into()) };
        match e.status(&unc, true) {
            MappingState::Unavailable { reason } => assert!(reason.contains("network"), "{reason}"),
            other => panic!("{other:?}"),
        }
        assert!(e.apply(&unc, Some(Resolution::ReplaceEmptyFolder)).is_err());
        // Missing Windows folder.
        let gone = FolderConfig { mode: MappingMode::Shared, windows_path: Some("D:\\Nope".into()) };
        assert!(matches!(e.status(&gone, true), MappingState::Unavailable { .. }));
        // A drive that does not exist on this machine.
        let drive = FolderConfig { mode: MappingMode::Shared, windows_path: Some("Q:\\Stuff".into()) };
        assert!(matches!(e.status(&drive, true), MappingState::Unavailable { .. }));
        assert!(e.apply(&drive, Some(Resolution::ReplaceEmptyFolder)).is_err());
        assert!(e.home("Downloads").is_dir() && !e.is_link("Downloads"));
        assert!(!e.distro.calls().iter().any(|c| c.starts_with("remove") || c.starts_with("make_symlink")));
    }

    #[test]
    fn a_known_folder_windows_does_not_report_is_unavailable() {
        let e = Env::new();
        let store = e.store();
        let s = e.with(|c| status(c, &store, KnownFolder::Music, &e.shared, true));
        match s.state {
            MappingState::Unavailable { reason } => assert!(reason.contains("did not report"), "{reason}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(s.windows_path, None);
    }

    #[test]
    fn disabled_automount_blocks_mapping() {
        let e = Env::new();
        let store = e.store();
        let exists = |_: &str| true;
        let s = {
            let ctx = Ctx {
                distro: "Debian",
                linux_root: &e.root,
                home: "/home/me",
                mount_root: "/mnt",
                automount: false,
                ops: &e.distro,
                drives: &Fixed,
                resolver: &e.folders,
                windows_dir_exists: &exists,
                now: 1,
            };
            status(&ctx, &store, KnownFolder::Downloads, &e.shared, true)
        };
        match s.state {
            MappingState::Unavailable { reason } => assert!(reason.contains("automount"), "{reason}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_custom_automount_root_is_used_for_the_target() {
        let e = Env::new();
        let store = e.store();
        let distro = FakeDistro::new(&e.root, "/home/me").mount("/windir/d", &e.win);
        let exists = |_: &str| true;
        let ctx = Ctx {
            distro: "Debian",
            linux_root: &e.root,
            home: "/home/me",
            mount_root: "/windir",
            automount: true,
            ops: &distro,
            drives: &Fixed,
            resolver: &e.folders,
            windows_dir_exists: &exists,
            now: 1,
        };
        apply(&ctx, &store, KnownFolder::Downloads, &e.shared, None).unwrap();
        assert_eq!(fs::read_link(e.home("Downloads")).unwrap(), e.win.join("Users/Someone/Downloads"));
        assert_eq!(store.find("Debian", "downloads").unwrap().target, "/windir/d/Users/Someone/Downloads");
    }

    #[test]
    fn a_failure_while_creating_the_link_rolls_back_an_empty_folder() {
        let e = Env::new();
        fs::create_dir(e.home("Downloads")).unwrap();
        e.distro.fail_on("make_symlink");
        let err = e.apply(&e.shared, Some(Resolution::ReplaceEmptyFolder)).unwrap_err();
        assert!(err.contains("empty folder was put back"), "{err}");
        assert!(e.home("Downloads").is_dir() && !e.is_link("Downloads"));
        assert!(e.store().all().is_empty());
    }

    #[test]
    fn a_failure_while_creating_the_link_rolls_back_a_move_aside() {
        let e = Env::new();
        fs::create_dir(e.home("Downloads")).unwrap();
        fs::write(e.home("Downloads/keep.txt"), "keep").unwrap();
        e.distro.fail_on("make_symlink");
        let err = e.apply(&e.shared, Some(Resolution::MoveAside)).unwrap_err();
        assert!(err.contains("moved back"), "{err}");
        assert_eq!(fs::read_to_string(e.home("Downloads/keep.txt")).unwrap(), "keep");
        assert!(!e.home("Downloads.vlinux-backup").exists());
        assert!(e.store().all().is_empty());
    }

    #[test]
    fn a_failure_while_replacing_a_link_restores_the_old_link() {
        let e = Env::new();
        fs::create_dir(e.root.join("elsewhere")).unwrap();
        std::os::unix::fs::symlink(e.root.join("elsewhere"), e.home("Downloads")).unwrap();
        // Only the *new* link fails; restoring the old one must still work, so fail once.
        struct Once<'a>(&'a FakeDistro, std::sync::atomic::AtomicBool);
        impl DistroOps for Once<'_> {
            fn home_dir(&self) -> Result<String, OpsError> {
                self.0.home_dir()
            }
            fn read_link(&self, p: &str) -> Result<Option<String>, OpsError> {
                self.0.read_link(p)
            }
            fn make_symlink(&self, t: &str, l: &str) -> Result<(), OpsError> {
                if !self.1.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    return Err(OpsError("boom".into()));
                }
                self.0.make_symlink(t, l)
            }
            fn remove_symlink(&self, l: &str) -> Result<(), OpsError> {
                self.0.remove_symlink(l)
            }
            fn remove_empty_dir(&self, p: &str) -> Result<(), OpsError> {
                self.0.remove_empty_dir(p)
            }
            fn make_dir(&self, p: &str) -> Result<(), OpsError> {
                self.0.make_dir(p)
            }
            fn rename_noclobber(&self, a: &str, b: &str) -> Result<(), OpsError> {
                self.0.rename_noclobber(a, b)
            }
            fn dir_exists(&self, p: &str) -> Result<bool, OpsError> {
                self.0.dir_exists(p)
            }
        }
        let once = Once(&e.distro, std::sync::atomic::AtomicBool::new(false));
        let store = e.store();
        let win = e.win.clone();
        let exists = move |p: &str| p.starts_with("D:\\") && win.join("Users/Someone/Downloads").is_dir();
        let ctx = Ctx {
            distro: "Debian",
            linux_root: &e.root,
            home: "/home/me",
            mount_root: "/mnt",
            automount: true,
            ops: &once,
            drives: &Fixed,
            resolver: &e.folders,
            windows_dir_exists: &exists,
            now: 1,
        };
        let err = apply(&ctx, &store, KnownFolder::Downloads, &e.shared, Some(Resolution::ReplaceLink)).unwrap_err();
        assert!(err.contains("previous link was restored"), "{err}");
        assert_eq!(fs::read_link(e.home("Downloads")).unwrap(), e.root.join("elsewhere"));
        assert!(store.all().is_empty());
    }

    #[test]
    fn unreadable_state_is_reported_not_guessed() {
        let e = Env::new();
        e.distro.fail_on("read_link");
        match e.status(&e.shared, true) {
            MappingState::Failed { message } => assert!(message.contains("Couldn't inspect"), "{message}"),
            other => panic!("{other:?}"),
        }
        assert!(e.apply(&e.shared, None).is_err());
        assert!(!e.is_link("Downloads"));
    }

    #[test]
    fn mappings_are_per_distribution() {
        let e = Env::new();
        e.apply(&e.shared, None).unwrap();
        let store = e.store();
        let other = store.find("Arch", "downloads");
        assert!(other.is_none());
        assert!(store.find("Debian", "downloads").is_some());
    }

    #[test]
    fn redirected_known_folders_follow_wherever_windows_says() {
        // Documents redirected to another drive-letter subtree: the engine only ever uses what the
        // resolver returns; there is no hard-coded profile path.
        let e = Env::new();
        let store = e.store();
        let s = e.with(|c| status(c, &store, KnownFolder::Documents, &e.shared, true));
        assert_eq!(s.windows_path.as_deref(), Some("D:\\Users\\Someone\\Documents"));
        assert_eq!(s.target.as_deref(), Some("/mnt/d/Users/Someone/Documents"));
        e.with(|c| apply(c, &store, KnownFolder::Documents, &e.shared, None)).unwrap();
        fs::write(e.home("Documents/a.docx"), "d").unwrap();
        assert!(e.win.join("Users/Someone/Documents/a.docx").exists());
    }
}
