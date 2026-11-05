//! Controlled file operations: create, rename, delete, copy and move.
//!
//! Safety rules enforced here (and tested):
//! * every location is resolved through [`Spaces`] first (validated, links followed one by one,
//!   Windows roots contained) — nothing here takes a raw path from the UI;
//! * **nothing is ever overwritten**: copy/move into an occupied name either keeps both (a
//!   "(copy)" name), skips, or reports a failure;
//! * delete needs an explicit confirmation flag, never follows links (a link is removed, its
//!   target untouched), and refuses the Linux home folder and top-level system folders;
//! * a move across file systems copies first and only removes the original when *everything* was
//!   copied; links inside copied folders are skipped (and reported), never followed;
//! * long operations report progress and can be cancelled between chunks.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::error::{FsError, FsErrorKind, FsResult};
use super::path::{check_name, split_ext};
use super::space::{EntryRef, Location, Spaces};
use crate::exec::CancelToken;

const MAX_DEPTH: u32 = 200;
const CHUNK: usize = 1 << 20;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Progress {
    pub files: u64,
    pub bytes: u64,
    pub current: String,
    pub skipped: u64,
    pub failed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ItemFailure {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct OpReport {
    /// Top-level items completed.
    pub items_done: u64,
    pub files: u64,
    pub bytes: u64,
    /// First few skipped items, with the reason.
    pub skipped: Vec<String>,
    pub failed: Vec<ItemFailure>,
    pub cancelled: bool,
    /// Where pasted/moved items ended up.
    pub results: Vec<Location>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferMode {
    Copy,
    Move,
}

/// What to do when the destination name is taken. There is deliberately no "overwrite".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Conflict {
    #[default]
    KeepBoth,
    Skip,
    Fail,
}

/// Progress, cancellation and the running report of one operation.
pub struct Runner<'a> {
    cancel: &'a CancelToken,
    on_progress: &'a mut dyn FnMut(&Progress),
    progress: Progress,
    report: OpReport,
    counting: bool,
}

enum Stop {
    Cancelled,
    /// Something below was skipped/failed and is already in the report.
    Partial,
    Io(io::Error),
}

impl From<io::Error> for Stop {
    fn from(e: io::Error) -> Self {
        Stop::Io(e)
    }
}

impl<'a> Runner<'a> {
    pub fn new(cancel: &'a CancelToken, on_progress: &'a mut dyn FnMut(&Progress)) -> Self {
        Self {
            cancel,
            on_progress,
            progress: Progress::default(),
            report: OpReport::default(),
            counting: true,
        }
    }

    pub fn finish(mut self) -> OpReport {
        self.report.cancelled = self.report.cancelled || self.cancel.is_cancelled();
        self.report
    }

    fn cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    fn emit(&mut self) {
        (self.on_progress)(&self.progress);
    }

    fn current(&mut self, name: &str) {
        self.progress.current = name.to_string();
        self.emit();
    }

    fn file_done(&mut self) {
        if self.counting {
            self.report.files += 1;
            self.progress.files += 1;
            self.emit();
        }
    }

    fn bytes(&mut self, n: u64) {
        if self.counting {
            self.report.bytes += n;
            self.progress.bytes += n;
            self.emit();
        }
    }

    fn skip(&mut self, what: &str, why: &str) {
        self.progress.skipped += 1;
        if self.report.skipped.len() < 50 {
            self.report.skipped.push(format!("{what}: {why}"));
        }
        self.emit();
    }

    fn fail(&mut self, path: &Path, e: &io::Error, action: &str) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        self.fail_msg(&name, FsError::from_io(e, action, &name).message);
    }

    fn fail_msg(&mut self, name: &str, message: String) {
        self.progress.failed += 1;
        if self.report.failed.len() < 50 {
            self.report.failed.push(ItemFailure {
                path: name.to_string(),
                message,
            });
        }
        self.emit();
    }
}

fn exists_nofollow(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok()
}

/// Folders whose removal or renaming would damage the distribution (or the user's whole home).
fn protect(spaces: &Spaces, e: &EntryRef) -> FsResult<()> {
    if let Location::Linux { path } = &e.location {
        if spaces.linux_home.as_deref() == Some(path.as_str()) {
            return Err(FsError::new(
                FsErrorKind::Refused,
                "That is your Linux home folder; it cannot be removed, moved or renamed from here.",
            ));
        }
        const SYSTEM: [&str; 18] = [
            "/bin", "/boot", "/dev", "/etc", "/home", "/lib", "/lib64", "/mnt", "/opt", "/proc",
            "/root", "/run", "/sbin", "/srv", "/sys", "/tmp", "/usr", "/var",
        ];
        if SYSTEM.contains(&path.as_str()) {
            return Err(FsError::new(
                FsErrorKind::Refused,
                format!(
                    "{path} is a system folder; it cannot be removed, moved or renamed from here."
                ),
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// create / rename
// ---------------------------------------------------------------------------------------------

pub fn create_dir(spaces: &Spaces, parent: &Location, name: &str) -> FsResult<Location> {
    check_name(name)?;
    let (parent_loc, _) = spaces.resolve_dir(parent)?;
    let entry = spaces.entry_in(parent_loc, name.to_string())?;
    fs::create_dir(&entry.host).map_err(|e| FsError::from_io(&e, "create the folder", name))?;
    Ok(entry.location)
}

pub fn create_file(spaces: &Spaces, parent: &Location, name: &str) -> FsResult<Location> {
    check_name(name)?;
    let (parent_loc, _) = spaces.resolve_dir(parent)?;
    let entry = spaces.entry_in(parent_loc, name.to_string())?;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&entry.host)
        .map_err(|e| FsError::from_io(&e, "create the file", name))?;
    Ok(entry.location)
}

pub fn rename(spaces: &Spaces, loc: &Location, new_name: &str) -> FsResult<Location> {
    check_name(new_name)?;
    let old = spaces.resolve_entry(loc)?;
    protect(spaces, &old)?;
    if old.name == new_name {
        return Ok(old.location);
    }
    let new = spaces.entry_in(old.parent.clone(), new_name.to_string())?;
    if !exists_nofollow(&old.host) {
        return Err(FsError::new(
            FsErrorKind::NotFound,
            format!("\"{}\" no longer exists. Refresh the folder.", old.name),
        ));
    }
    let case_only = old.name.eq_ignore_ascii_case(new_name);
    if exists_nofollow(&new.host) && !(case_only && cfg!(windows)) {
        return Err(FsError::new(
            FsErrorKind::AlreadyExists,
            format!("\"{new_name}\" already exists in this folder."),
        ));
    }
    fs::rename(&old.host, &new.host).map_err(|e| FsError::from_io(&e, "rename", &old.name))?;
    Ok(new.location)
}

// ---------------------------------------------------------------------------------------------
// delete
// ---------------------------------------------------------------------------------------------

fn remove_link(p: &Path) -> io::Result<()> {
    fs::remove_file(p).or_else(|_| fs::remove_dir(p))
}

/// Removes a symbolic link from INSIDE the distribution (`unlink`, which cannot remove a directory
/// and never follows a link). Used FIRST for links on the distribution's file share, because Windows
/// delete semantics for links on `\\wsl.localhost` are unreliable (measured on a real Arch).
pub trait LinkRemover {
    fn remove_link(&self, linux_path: &str) -> Result<(), String>;
}

/// Adapts the allow-listed in-distribution operations ([`DistroOps::remove_symlink`]).
pub struct DistroLinkRemover<'a>(pub &'a dyn crate::shared::DistroOps);

impl LinkRemover for DistroLinkRemover<'_> {
    fn remove_link(&self, linux_path: &str) -> Result<(), String> {
        self.0.remove_symlink(linux_path).map_err(|e| e.to_string())
    }
}

/// Removes one link from the Windows side. Injectable so tests can model what Windows really does
/// with links on the WSL share (the lab saw it fail with "path not found", "directory name is
/// invalid", or report success without removing anything).
type HostRemove<'a> = dyn Fn(&Path) -> io::Result<()> + 'a;

struct DelCtx<'a> {
    root: Option<&'a Path>,
    remover: Option<&'a dyn LinkRemover>,
    host_remove: &'a HostRemove<'a>,
}

impl DelCtx<'_> {
    /// The Linux path of a host path under the distribution root.
    fn linux_path(&self, host: &Path) -> Option<String> {
        let rel = host.strip_prefix(self.root?).ok()?;
        let mut out = String::new();
        for c in rel.components() {
            match c {
                std::path::Component::Normal(n) => {
                    out.push('/');
                    out.push_str(n.to_str()?);
                }
                _ => return None,
            }
        }
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    }

    fn in_linux_space(&self, host: &Path) -> bool {
        self.linux_path(host).is_some()
    }
}

fn vanished(p: &Path) -> bool {
    matches!(fs::symlink_metadata(p), Err(e) if e.kind() == io::ErrorKind::NotFound)
}

/// The directory listing is what the file manager shows, and the share may need a moment to
/// reflect a change made inside the distribution: look again briefly before giving up.
fn vanishes_soon(p: &Path) -> bool {
    for i in 0..10 {
        if vanished(p) {
            return true;
        }
        if i < 9 {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
    false
}

fn describe_io(e: &io::Error) -> String {
    // The standard text already ends in "(os error N)" when the system gave a code.
    e.to_string()
}

/// A rough, privacy-safe place name for a Linux path: only its first folder.
fn path_category(linux: Option<&str>) -> &'static str {
    match linux.and_then(|p| p.split('/').nth(1)) {
        Some("tmp") => "under /tmp",
        Some("home") => "in a home folder",
        Some("mnt") => "under /mnt",
        Some("root") => "under /root",
        Some(_) => "elsewhere in the distribution",
        None => "in a Windows folder",
    }
}

/// Remove ONE link (or other pointer-like entry), never what it points to. `path` was classified as
/// a link without following it. Findings from the real Arch share (see docs/VALIDATION.md): Windows
/// can fail to delete a WSL link ("path not found", "directory name is invalid") or even report
/// success while leaving it in place; `unlink` inside the distribution works. So on the share
/// the distribution removes the link first, Windows is only a last resort, and the result is
/// ALWAYS verified by looking again — never assumed from a return code. Every failure says what was
/// tried, what each side answered and what is still there.
fn remove_symlink_entry(path: &Path, ctx: &DelCtx<'_>) -> io::Result<()> {
    let linux = ctx.linux_path(path);
    let mut distro_note = "not used".to_string();
    if let (Some(remover), Some(l)) = (ctx.remover, linux.as_deref()) {
        distro_note = match remover.remove_link(l) {
            Ok(()) => "unlink ran without error".to_string(),
            Err(msg) => format!("unlink failed: {msg}"),
        };
        if vanishes_soon(path) {
            return Ok(());
        }
    }
    let windows_note = match (ctx.host_remove)(path) {
        Ok(()) => "reported success".to_string(),
        Err(e) => format!("failed: {}", describe_io(&e)),
    };
    if vanishes_soon(path) {
        return Ok(());
    }
    // Nothing worked. Say exactly what is still there (without following anything).
    let target_ok = match fs::metadata(path) {
        Ok(_) => "yes",
        Err(e) if e.kind() == io::ErrorKind::NotFound => "no or not reachable",
        Err(_) => "unknown",
    };
    Err(io::Error::other(format!(
        "could not remove a link {} (operation: remove one link only, never its target). In the distribution: {distro_note}. From Windows: {windows_note}. The link is still there: yes. What it points to is still reachable: {target_ok}. Nothing else was changed.",
        path_category(linux.as_deref())
    )))
}

#[cfg(windows)]
fn reparse_flags(m: &fs::Metadata) -> (bool, bool) {
    use std::os::windows::fs::MetadataExt;
    const DIRECTORY: u32 = 0x10;
    const REPARSE_POINT: u32 = 0x400;
    let a = m.file_attributes();
    (a & REPARSE_POINT != 0, a & DIRECTORY != 0)
}
#[cfg(not(windows))]
fn reparse_flags(_m: &fs::Metadata) -> (bool, bool) {
    (false, false)
}

/// Whether an entry is a link — or any other "entry that is really a pointer" — and so is removed
/// as ONE item and NEVER descended into or followed.
///
/// * `is_symlink()` (a name-surrogate reparse tag).
/// * Any reparse-point folder, whatever its tag (a junction, a mount point).
/// * Inside the distribution's share, ANY reparse point: the lab showed WSL links appear to Windows
///   as plain "ReparsePoint" entries (not even flagged as folders), and nothing but a link, socket
///   or pipe is a reparse point there.
fn linkish(m: &fs::Metadata, linux_space: bool) -> bool {
    let (reparse, dir) = reparse_flags(m);
    m.file_type().is_symlink() || (reparse && (dir || linux_space))
}

/// The metadata of `path` as its PARENT's directory enumeration reports it (the same source the
/// file listing uses, which never follows links). `None` if it cannot be found.
fn enumerated_meta(path: &Path) -> Option<fs::Metadata> {
    let parent = path.parent()?;
    let name = path.file_name()?;
    for entry in fs::read_dir(parent).ok()?.take(1_000_000) {
        let entry = entry.ok()?;
        if entry.file_name() == name {
            return entry.metadata().ok();
        }
    }
    None
}

fn remove_any(
    path: &Path,
    run: &mut Runner<'_>,
    depth: u32,
    ctx: &DelCtx<'_>,
    seen: Option<fs::Metadata>,
) -> Result<(), Stop> {
    if run.cancelled() {
        return Err(Stop::Cancelled);
    }
    let linux_space = ctx.in_linux_space(path);
    let listed_as_link = seen.as_ref().is_some_and(|x| linkish(x, linux_space));
    // Some link kinds on the share cannot even be opened for their own metadata from Windows; if
    // the directory listing already says it is a link, that is enough to treat it as one.
    let m = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(_) if listed_as_link => {
            remove_symlink_entry(path, ctx)?;
            run.file_done();
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };
    let ft = m.file_type();
    if linkish(&m, linux_space) || listed_as_link {
        remove_symlink_entry(path, ctx)?;
        run.file_done();
        return Ok(());
    }
    if ft.is_dir() {
        if depth > MAX_DEPTH {
            return Err(Stop::Io(io::Error::other("the folder nesting is too deep")));
        }
        let mut partial = false;
        for child in fs::read_dir(path)? {
            let child = child?;
            match remove_any(&child.path(), run, depth + 1, ctx, child.metadata().ok()) {
                Ok(()) => {}
                Err(Stop::Cancelled) => return Err(Stop::Cancelled),
                Err(Stop::Partial) => partial = true,
                Err(Stop::Io(e)) => {
                    run.fail(&child.path(), &e, "delete");
                    partial = true;
                }
            }
        }
        if partial {
            return Err(Stop::Partial);
        }
        fs::remove_dir(path)?;
        run.file_done();
        Ok(())
    } else {
        fs::remove_file(path)?;
        run.file_done();
        Ok(())
    }
}

/// Delete `targets` (permanently). `confirmed` must be true: the UI sets it only after the user
/// confirmed in a dialog that named what is about to be removed.
pub fn delete(
    spaces: &Spaces,
    targets: &[Location],
    confirmed: bool,
    run: &mut Runner<'_>,
) -> FsResult<()> {
    delete_with(spaces, targets, confirmed, run, None)
}

/// [`delete`] with an optional in-distribution fallback for removing symbolic links.
pub fn delete_with(
    spaces: &Spaces,
    targets: &[Location],
    confirmed: bool,
    run: &mut Runner<'_>,
    remover: Option<&dyn LinkRemover>,
) -> FsResult<()> {
    delete_with_host(spaces, targets, confirmed, run, remover, &remove_link)
}

/// [`delete_with`] with the Windows-side link removal injected (tests model the real share).
fn delete_with_host(
    spaces: &Spaces,
    targets: &[Location],
    confirmed: bool,
    run: &mut Runner<'_>,
    remover: Option<&dyn LinkRemover>,
    host_remove: &HostRemove<'_>,
) -> FsResult<()> {
    let ctx = DelCtx {
        root: spaces.linux_root.as_deref(),
        remover,
        host_remove,
    };
    if !confirmed {
        return Err(FsError::new(
            FsErrorKind::Refused,
            "Deleting needs confirmation. Nothing was deleted.",
        ));
    }
    if targets.is_empty() {
        return Err(FsError::new(
            FsErrorKind::InvalidPath,
            "Nothing was selected.",
        ));
    }
    // Validate everything first so a bad item cannot leave a half-done batch.
    let mut entries = Vec::new();
    for t in targets {
        let e = spaces.resolve_entry(t)?;
        protect(spaces, &e)?;
        entries.push(e);
    }
    for e in entries {
        if run.cancelled() {
            run.report.cancelled = true;
            break;
        }
        run.current(&e.name);
        match remove_any(&e.host, run, 0, &ctx, enumerated_meta(&e.host)) {
            Ok(()) => run.report.items_done += 1,
            Err(Stop::Cancelled) => {
                run.report.cancelled = true;
                break;
            }
            Err(Stop::Partial) => {}
            Err(Stop::Io(err)) => run.fail(&e.host, &err, "delete"),
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// copy / move
// ---------------------------------------------------------------------------------------------

fn copy_file(src: &Path, dst: &Path, run: &mut Runner<'_>) -> Result<(), Stop> {
    let mut input = fs::File::open(src)?;
    // `create_new`: an existing file is never overwritten, even if it appeared a moment ago.
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dst)?;
    let mut buf = vec![0u8; CHUNK];
    let result: Result<(), Stop> = loop {
        if run.cancelled() {
            break Err(Stop::Cancelled);
        }
        let n = match input.read(&mut buf) {
            Ok(0) => break Ok(()),
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => break Err(e.into()),
        };
        if let Err(e) = output.write_all(&buf[..n]) {
            break Err(e.into());
        }
        run.bytes(n as u64);
    };
    drop(output);
    match result {
        Ok(()) => {
            if let Ok(m) = fs::metadata(src) {
                let _ = fs::set_permissions(dst, m.permissions());
            }
            run.file_done();
            Ok(())
        }
        Err(e) => {
            // The half-written file is ours (we just created it): remove it.
            let _ = fs::remove_file(dst);
            Err(e)
        }
    }
}

fn copy_tree(src: &Path, dst: &Path, run: &mut Runner<'_>, depth: u32) -> Result<(), Stop> {
    if run.cancelled() {
        return Err(Stop::Cancelled);
    }
    let m = fs::symlink_metadata(src)?;
    let ft = m.file_type();
    let name = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if linkish(&m, false) {
        run.skip(&name, "a link (links are not copied)");
        return Err(Stop::Partial);
    }
    if ft.is_dir() {
        if depth > MAX_DEPTH {
            return Err(Stop::Io(io::Error::other("the folder nesting is too deep")));
        }
        fs::create_dir(dst)?;
        let mut partial = false;
        for child in fs::read_dir(src)? {
            let child = child?;
            match copy_tree(&child.path(), &dst.join(child.file_name()), run, depth + 1) {
                Ok(()) => {}
                Err(Stop::Cancelled) => return Err(Stop::Cancelled),
                Err(Stop::Partial) => partial = true,
                Err(Stop::Io(e)) => {
                    run.fail(&child.path(), &e, "copy");
                    partial = true;
                }
            }
        }
        if partial {
            Err(Stop::Partial)
        } else {
            Ok(())
        }
    } else if ft.is_file() {
        run.current(&name);
        copy_file(src, dst, run)
    } else {
        run.skip(&name, "a special file");
        Err(Stop::Partial)
    }
}

/// Copy, then remove the original — but only if every item was copied.
fn move_by_copy(src: &Path, dst: &Path, run: &mut Runner<'_>) -> Result<(), Stop> {
    copy_tree(src, dst, run, 0)?;
    let was = run.counting;
    run.counting = false;
    let ctx = DelCtx {
        root: None,
        remover: None,
        host_remove: &remove_link,
    };
    let removed = remove_any(src, run, 0, &ctx, enumerated_meta(src));
    run.counting = was;
    removed
}

fn move_one(src: &Path, dst: &Path, run: &mut Runner<'_>) -> Result<(), Stop> {
    match fs::rename(src, dst) {
        Ok(()) => {
            run.file_done();
            Ok(())
        }
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::PermissionDenied
                    | io::ErrorKind::NotFound
                    | io::ErrorKind::AlreadyExists
            ) =>
        {
            Err(e.into())
        }
        // Typically "different file system" (Linux <-> Windows, or across mounts).
        Err(_) => match move_by_copy(src, dst, run) {
            Err(Stop::Partial) => {
                // Something was left behind: the original is intact, say so.
                run.fail_msg(
                    &src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                    "Copied what could be copied, but the original was kept because not everything could be moved.".into(),
                );
                Err(Stop::Partial)
            }
            other => other,
        },
    }
}

/// "name (copy).ext", "name (copy 2).ext", ...
fn unique_name(dir: &Path, name: &str, is_dir: bool) -> Option<String> {
    let (stem, ext) = if is_dir { (name, "") } else { split_ext(name) };
    for n in 1..10_000u32 {
        let candidate = if n == 1 {
            format!("{stem} (copy){ext}")
        } else {
            format!("{stem} (copy {n}){ext}")
        };
        if !exists_nofollow(&dir.join(&candidate)) {
            return Some(candidate);
        }
    }
    None
}

pub fn transfer(
    spaces: &Spaces,
    sources: &[Location],
    dest: &Location,
    mode: TransferMode,
    conflict: Conflict,
    run: &mut Runner<'_>,
) -> FsResult<()> {
    if sources.is_empty() {
        return Err(FsError::new(
            FsErrorKind::InvalidPath,
            "Nothing was selected.",
        ));
    }
    let (dest_loc, dest_host) = spaces.resolve_dir(dest)?;
    let dest_meta = fs::metadata(&dest_host)
        .map_err(|e| FsError::from_io(&e, "open the destination folder", "destination"))?;
    if !dest_meta.is_dir() {
        return Err(FsError::new(
            FsErrorKind::NotADirectory,
            "The destination is not a folder.",
        ));
    }
    let mut jobs: Vec<(EntryRef, fs::Metadata)> = Vec::new();
    for s in sources {
        let e = spaces.resolve_entry(s)?;
        if mode == TransferMode::Move {
            protect(spaces, &e)?;
        }
        let meta =
            fs::symlink_metadata(&e.host).map_err(|err| FsError::from_io(&err, "find", &e.name))?;
        if meta.is_dir()
            && !meta.file_type().is_symlink()
            && (dest_host == e.host || dest_host.starts_with(&e.host))
        {
            return Err(FsError::new(
                FsErrorKind::InvalidPath,
                format!("\"{}\" cannot be placed inside itself.", e.name),
            ));
        }
        jobs.push((e, meta));
    }

    for (e, meta) in jobs {
        if run.cancelled() {
            run.report.cancelled = true;
            break;
        }
        let is_dir = meta.is_dir() && !meta.file_type().is_symlink();
        if mode == TransferMode::Move && e.host.parent() == Some(dest_host.as_path()) {
            run.skip(&e.name, "it is already in that folder");
            continue;
        }
        let target_name = if !exists_nofollow(&dest_host.join(&e.name)) {
            e.name.clone()
        } else {
            match conflict {
                Conflict::KeepBoth => match unique_name(&dest_host, &e.name, is_dir) {
                    Some(n) => n,
                    None => {
                        run.fail_msg(&e.name, "Could not find a free name.".into());
                        continue;
                    }
                },
                Conflict::Skip => {
                    run.skip(&e.name, "an item with that name already exists");
                    continue;
                }
                Conflict::Fail => {
                    run.fail_msg(
                        &e.name,
                        format!("\"{}\" already exists in the destination.", e.name),
                    );
                    continue;
                }
            }
        };
        let dst: PathBuf = dest_host.join(&target_name);
        run.current(&e.name);
        let r = match mode {
            TransferMode::Copy => copy_tree(&e.host, &dst, run, 0),
            TransferMode::Move => move_one(&e.host, &dst, run),
        };
        match r {
            Ok(()) => {
                run.report.items_done += 1;
                if let Ok(entry) = spaces.entry_in(dest_loc.clone(), target_name) {
                    run.report.results.push(entry.location);
                }
            }
            Err(Stop::Cancelled) => {
                run.report.cancelled = true;
                break;
            }
            Err(Stop::Partial) => {}
            Err(Stop::Io(err)) => run.fail(
                &e.host,
                &err,
                if mode == TransferMode::Copy {
                    "copy"
                } else {
                    "move"
                },
            ),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::space::WindowsRoot;
    use super::super::testutil::TempDir;
    use super::*;

    fn spaces(linux: &TempDir, win: &TempDir) -> Spaces {
        Spaces {
            linux_root: Some(linux.path().to_path_buf()),
            linux_home: Some("/home/me".into()),
            mount_root: "/mnt".into(),
            windows: vec![WindowsRoot {
                id: "documents".into(),
                label: "Documents".into(),
                dir: win.path().to_path_buf(),
                linux_mount: None,
            }],
            mapped_links: vec![],
        }
    }

    fn run_with<T>(f: impl FnOnce(&mut Runner<'_>) -> T) -> (T, OpReport, usize) {
        let cancel = CancelToken::new();
        let mut events = 0usize;
        let mut sink = |_: &Progress| events += 1;
        let mut r = Runner::new(&cancel, &mut sink);
        let out = f(&mut r);
        let report = r.finish();
        (out, report, events)
    }

    fn lin(p: &str) -> Location {
        Location::linux(p)
    }

    #[test]
    fn create_folder_and_file() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        l.dir("home/me");
        let s = spaces(&l, &w);
        let d = create_dir(&s, &lin("/home/me"), "New folder").unwrap();
        assert_eq!(d, lin("/home/me/New folder"));
        assert!(l.join("home/me/New folder").is_dir());
        let f = create_file(&s, &lin("/home/me"), "notes.txt").unwrap();
        assert_eq!(f, lin("/home/me/notes.txt"));
        assert_eq!(fs::read_to_string(l.join("home/me/notes.txt")).unwrap(), "");
    }

    #[test]
    fn create_refuses_bad_names_and_existing_items() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        l.file("home/me/exists.txt", "keep me");
        let s = spaces(&l, &w);
        for bad in ["", "a/b", "..", "x:y", "nul"] {
            assert_eq!(
                create_file(&s, &lin("/home/me"), bad).unwrap_err().kind,
                FsErrorKind::InvalidName,
                "{bad}"
            );
        }
        let e = create_file(&s, &lin("/home/me"), "exists.txt").unwrap_err();
        assert_eq!(e.kind, FsErrorKind::AlreadyExists);
        assert_eq!(
            fs::read_to_string(l.join("home/me/exists.txt")).unwrap(),
            "keep me",
            "an existing file is never truncated"
        );
        assert_eq!(
            create_dir(&s, &lin("/home/me"), "exists.txt")
                .unwrap_err()
                .kind,
            FsErrorKind::AlreadyExists
        );
    }

    #[test]
    fn create_in_a_missing_folder_is_not_found() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        let s = spaces(&l, &w);
        assert_eq!(
            create_dir(&s, &lin("/nope/deeper"), "x").unwrap_err().kind,
            FsErrorKind::NotFound
        );
    }

    #[test]
    fn rename_files_and_folders() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        l.file("home/me/a.txt", "A");
        l.file("home/me/b.txt", "B");
        l.dir("home/me/dir");
        let s = spaces(&l, &w);
        assert_eq!(
            rename(&s, &lin("/home/me/a.txt"), "c.txt").unwrap(),
            lin("/home/me/c.txt")
        );
        assert!(!l.join("home/me/a.txt").exists() && l.join("home/me/c.txt").exists());
        assert_eq!(
            rename(&s, &lin("/home/me/dir"), "folder").unwrap(),
            lin("/home/me/folder")
        );
        // Never over an existing name.
        assert_eq!(
            rename(&s, &lin("/home/me/c.txt"), "b.txt")
                .unwrap_err()
                .kind,
            FsErrorKind::AlreadyExists
        );
        assert_eq!(fs::read_to_string(l.join("home/me/b.txt")).unwrap(), "B");
        // Same name is a no-op; bad names and the root are refused.
        assert_eq!(
            rename(&s, &lin("/home/me/b.txt"), "b.txt").unwrap(),
            lin("/home/me/b.txt")
        );
        assert_eq!(
            rename(&s, &lin("/home/me/b.txt"), "a/b").unwrap_err().kind,
            FsErrorKind::InvalidName
        );
        assert_eq!(
            rename(&s, &lin("/"), "x").unwrap_err().kind,
            FsErrorKind::Refused
        );
        assert_eq!(
            rename(&s, &lin("/home/me/missing"), "x").unwrap_err().kind,
            FsErrorKind::NotFound
        );
        assert_eq!(
            rename(&s, &lin("/home/me"), "renamed").unwrap_err().kind,
            FsErrorKind::Refused,
            "the home folder is protected"
        );
    }

    #[test]
    fn rename_in_a_windows_folder() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        w.file("old.txt", "x");
        let s = spaces(&l, &w);
        let r = rename(&s, &Location::windows("documents", &["old.txt"]), "new.txt").unwrap();
        assert_eq!(r, Location::windows("documents", &["new.txt"]));
        assert!(w.join("new.txt").exists() && !w.join("old.txt").exists());
        assert_eq!(
            rename(&s, &Location::windows("documents", &[]), "x")
                .unwrap_err()
                .kind,
            FsErrorKind::Refused
        );
    }

    #[test]
    fn delete_requires_confirmation() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        let f = l.file("home/me/keep.txt", "x");
        let s = spaces(&l, &w);
        let (r, rep, _) = run_with(|run| delete(&s, &[lin("/home/me/keep.txt")], false, run));
        assert_eq!(r.unwrap_err().kind, FsErrorKind::Refused);
        assert!(f.exists(), "nothing is deleted without confirmation");
        assert_eq!(rep.items_done, 0);
    }

    #[test]
    fn delete_removes_files_and_whole_folders_and_counts() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        l.file("home/me/proj/a.txt", "1");
        l.file("home/me/proj/sub/b.txt", "22");
        l.file("home/me/single.txt", "x");
        let s = spaces(&l, &w);
        let (r, rep, events) = run_with(|run| {
            delete(
                &s,
                &[lin("/home/me/proj"), lin("/home/me/single.txt")],
                true,
                run,
            )
        });
        r.unwrap();
        assert!(!l.join("home/me/proj").exists() && !l.join("home/me/single.txt").exists());
        assert_eq!(rep.items_done, 2);
        assert_eq!(rep.files, 5, "2 files + 2 folders + 1 file");
        assert!(rep.failed.is_empty());
        assert!(events > 0, "progress is reported");
    }

    #[test]
    fn delete_validates_everything_before_removing_anything() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        let f = l.file("home/me/ok.txt", "x");
        let s = spaces(&l, &w);
        let (r, _, _) = run_with(|run| {
            delete(
                &s,
                &[lin("/home/me/ok.txt"), lin("/home/me/../etc")],
                true,
                run,
            )
        });
        assert_eq!(r.unwrap_err().kind, FsErrorKind::InvalidPath);
        assert!(f.exists());
    }

    #[test]
    fn delete_refuses_home_root_and_system_folders() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        l.dir("home/me");
        l.dir("etc");
        l.dir("usr");
        let s = spaces(&l, &w);
        for p in ["/", "/home/me", "/etc", "/usr", "/home"] {
            let (r, _, _) = run_with(|run| delete(&s, &[lin(p)], true, run));
            assert_eq!(r.unwrap_err().kind, FsErrorKind::Refused, "{p}");
        }
        assert!(l.join("home/me").exists() && l.join("etc").exists());
        let (r, _, _) =
            run_with(|run| delete(&s, &[Location::windows("documents", &[])], true, run));
        assert_eq!(
            r.unwrap_err().kind,
            FsErrorKind::Refused,
            "a shared folder root cannot be deleted"
        );
    }

    #[test]
    fn delete_of_a_missing_item_is_reported_not_fatal() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        l.dir("home/me");
        let s = spaces(&l, &w);
        let (r, rep, _) = run_with(|run| delete(&s, &[lin("/home/me/ghost.txt")], true, run));
        r.unwrap();
        assert_eq!(rep.failed.len(), 1);
        assert!(rep.failed[0].message.contains("ghost.txt"));
        assert_eq!(rep.items_done, 0);
    }

    #[cfg(unix)]
    #[test]
    fn delete_never_follows_links() {
        use std::os::unix::fs::symlink;
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        let outside = TempDir::new("outside");
        outside.file("precious.txt", "do not delete");
        l.dir("home/me/folder");
        symlink(outside.path(), l.join("home/me/folder/link")).unwrap();
        symlink(outside.path(), l.join("home/me/toplink")).unwrap();
        let s = spaces(&l, &w);
        let (r, rep, _) = run_with(|run| {
            delete(
                &s,
                &[lin("/home/me/folder"), lin("/home/me/toplink")],
                true,
                run,
            )
        });
        r.unwrap();
        assert!(
            !l.join("home/me/folder").exists()
                && fs::symlink_metadata(l.join("home/me/toplink")).is_err()
        );
        assert_eq!(
            fs::read_to_string(outside.join("precious.txt")).unwrap(),
            "do not delete"
        );
        assert!(rep.failed.is_empty());
    }

    // ---- M6.2: link deletion (real diagnostic: "Deleting the link reported errors") --------

    #[cfg(unix)]
    #[test]
    fn file_directory_and_dangling_links_are_removed_and_their_targets_survive() {
        use std::os::unix::fs::symlink;
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        l.file("home/me/target-dir/inside.txt", "inside");
        l.file("home/me/target-file.txt", "file");
        symlink(l.join("home/me/target-dir"), l.join("home/me/dir-link")).unwrap();
        symlink(
            l.join("home/me/target-file.txt"),
            l.join("home/me/file-link"),
        )
        .unwrap();
        symlink(l.join("home/me/does-not-exist"), l.join("home/me/dangling")).unwrap();
        let s = spaces(&l, &w);
        let (r, rep, _) = run_with(|run| {
            delete(
                &s,
                &[
                    lin("/home/me/dir-link"),
                    lin("/home/me/file-link"),
                    lin("/home/me/dangling"),
                ],
                true,
                run,
            )
        });
        r.unwrap();
        assert!(rep.failed.is_empty(), "{:?}", rep.failed);
        assert_eq!(rep.items_done, 3);
        for gone in ["dir-link", "file-link", "dangling"] {
            assert!(
                fs::symlink_metadata(l.join(&format!("home/me/{gone}"))).is_err(),
                "{gone}"
            );
        }
        assert_eq!(
            fs::read_to_string(l.join("home/me/target-dir/inside.txt")).unwrap(),
            "inside"
        );
        assert_eq!(
            fs::read_to_string(l.join("home/me/target-file.txt")).unwrap(),
            "file"
        );
    }

    /// Makes `dir` unwritable for this (non-root) user so removing an entry in it fails on the host
    /// side, and restores it when dropped. `None` if the process can write anyway (root).
    #[cfg(unix)]
    struct Locked(PathBuf);
    #[cfg(unix)]
    impl Locked {
        fn new(dir: PathBuf) -> Option<Locked> {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
            let probe = dir.join(".probe");
            if fs::write(&probe, "").is_ok() {
                let _ = fs::remove_file(&probe);
                fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
                return None;
            }
            Some(Locked(dir))
        }
        fn unlock(&self) {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
        }
    }
    #[cfg(unix)]
    impl Drop for Locked {
        fn drop(&mut self) {
            self.unlock();
        }
    }

    /// Stands in for `unlink` inside the distribution: records the Linux path it was asked for.
    #[cfg(unix)]
    struct FakeUnlink<'a> {
        calls: std::cell::RefCell<Vec<String>>,
        root: PathBuf,
        lock: Option<&'a Locked>,
        succeed: bool,
        really_remove: bool,
    }
    #[cfg(unix)]
    impl LinkRemover for FakeUnlink<'_> {
        fn remove_link(&self, linux_path: &str) -> Result<(), String> {
            self.calls.borrow_mut().push(linux_path.to_string());
            if !self.succeed {
                return Err("Permission denied".into());
            }
            if self.really_remove {
                if let Some(l) = self.lock {
                    l.unlock();
                }
                let mut p = self.root.clone();
                for c in linux_path.split('/').filter(|c| !c.is_empty()) {
                    p.push(c);
                }
                fs::remove_file(p).map_err(|e| e.to_string())?;
            }
            Ok(())
        }
    }

    #[cfg(unix)]
    fn locked_link_world(l: &TempDir) -> Option<Locked> {
        use std::os::unix::fs::symlink;
        l.file("home/me/target-dir/inside.txt", "inside");
        l.dir("home/me/links");
        symlink(
            l.join("home/me/target-dir"),
            l.join("home/me/links/dir-link"),
        )
        .unwrap();
        Locked::new(l.join("home/me/links"))
    }

    #[cfg(unix)]
    #[test]
    fn a_link_windows_cannot_remove_is_unlinked_inside_the_distribution() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        let Some(lock) = locked_link_world(&l) else {
            return;
        };
        let s = spaces(&l, &w);
        let rm = FakeUnlink {
            calls: Default::default(),
            root: l.path().to_path_buf(),
            lock: Some(&lock),
            succeed: true,
            really_remove: true,
        };
        let (r, rep, _) = run_with(|run| {
            delete_with(&s, &[lin("/home/me/links/dir-link")], true, run, Some(&rm))
        });
        r.unwrap();
        assert!(rep.failed.is_empty(), "{:?}", rep.failed);
        assert_eq!(rep.items_done, 1);
        assert_eq!(
            *rm.calls.borrow(),
            vec!["/home/me/links/dir-link".to_string()]
        );
        assert!(fs::symlink_metadata(l.join("home/me/links/dir-link")).is_err());
        assert_eq!(
            fs::read_to_string(l.join("home/me/target-dir/inside.txt")).unwrap(),
            "inside"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_link_nobody_can_remove_is_reported_with_both_reasons_and_nothing_else_is_touched() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        let Some(lock) = locked_link_world(&l) else {
            return;
        };
        let s = spaces(&l, &w);
        for (succeed, expect) in [
            (false, "unlink failed: Permission denied"),
            (true, "unlink ran without error"),
        ] {
            let rm = FakeUnlink {
                calls: Default::default(),
                root: l.path().to_path_buf(),
                lock: Some(&lock),
                succeed,
                really_remove: false,
            };
            let (r, rep, _) = run_with(|run| {
                delete_with(&s, &[lin("/home/me/links/dir-link")], true, run, Some(&rm))
            });
            r.unwrap();
            assert_eq!(rep.items_done, 0);
            assert_eq!(rep.failed.len(), 1, "{:?}", rep.failed);
            let m = &rep.failed[0].message;
            assert!(m.contains(expect), "{m}");
            // Everything an engineer needs: what was tried, both answers, what remains.
            assert!(m.contains("From Windows: failed:"), "{m}");
            assert!(m.contains("The link is still there: yes"), "{m}");
            assert!(
                m.contains("under /home") || m.contains("in a home folder"),
                "{m}"
            );
            assert!(fs::symlink_metadata(l.join("home/me/links/dir-link")).is_ok());
            assert_eq!(
                fs::read_to_string(l.join("home/me/target-dir/inside.txt")).unwrap(),
                "inside"
            );
        }
        // Without a fallback the host error is reported unchanged, never hidden.
        let (_, rep, _) = run_with(|run| delete(&s, &[lin("/home/me/links/dir-link")], true, run));
        assert_eq!(rep.failed.len(), 1);
        assert!(fs::symlink_metadata(l.join("home/me/links/dir-link")).is_ok());
    }

    // ---- M6.3: what the REAL share does (measured on a real Arch, WSL 3.0.3.0), modelled -------
    //
    // MODELLED tests: the lab observed that Windows deleting a link on \\wsl.localhost fails with
    // "path not found"/"directory name is invalid", or reports success while leaving the link, and
    // that `unlink` inside the distribution works. These tests inject those behaviours.

    #[cfg(unix)]
    fn plain_link_world(l: &TempDir) {
        use std::os::unix::fs::symlink;
        l.file("home/me/target-dir/inside.txt", "inside");
        l.dir("home/me/links");
        symlink(
            l.join("home/me/target-dir"),
            l.join("home/me/links/dir-link"),
        )
        .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn on_the_share_the_distribution_removes_the_link_first_and_windows_is_not_asked() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        plain_link_world(&l);
        let s = spaces(&l, &w);
        let rm = FakeUnlink {
            calls: Default::default(),
            root: l.path().to_path_buf(),
            lock: None,
            succeed: true,
            really_remove: true,
        };
        let host_calls = std::cell::Cell::new(0);
        let host = |p: &Path| -> io::Result<()> {
            host_calls.set(host_calls.get() + 1);
            remove_link(p)
        };
        let (r, rep, _) = run_with(|run| {
            delete_with_host(
                &s,
                &[lin("/home/me/links/dir-link")],
                true,
                run,
                Some(&rm),
                &host,
            )
        });
        r.unwrap();
        assert!(rep.failed.is_empty(), "{:?}", rep.failed);
        assert_eq!(rep.items_done, 1);
        assert_eq!(
            *rm.calls.borrow(),
            vec!["/home/me/links/dir-link".to_string()]
        );
        assert_eq!(
            host_calls.get(),
            0,
            "Windows must not be asked when the distribution succeeded"
        );
        assert_eq!(
            fs::read_to_string(l.join("home/me/target-dir/inside.txt")).unwrap(),
            "inside"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_windows_delete_that_reports_success_without_removing_anything_is_not_believed() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        plain_link_world(&l);
        let s = spaces(&l, &w);
        let rm = FakeUnlink {
            calls: Default::default(),
            root: l.path().to_path_buf(),
            lock: None,
            succeed: false,
            really_remove: false,
        };
        // The lab: `cmd del` and [IO.File]::Delete on a dangling link said "ok", the link stayed.
        let liar = |_: &Path| -> io::Result<()> { Ok(()) };
        let (r, rep, _) = run_with(|run| {
            delete_with_host(
                &s,
                &[lin("/home/me/links/dir-link")],
                true,
                run,
                Some(&rm),
                &liar,
            )
        });
        r.unwrap();
        assert_eq!(rep.items_done, 0);
        assert_eq!(rep.failed.len(), 1);
        let m = &rep.failed[0].message;
        assert!(m.contains("From Windows: reported success"), "{m}");
        assert!(m.contains("unlink failed: Permission denied"), "{m}");
        assert!(m.contains("The link is still there: yes"), "{m}");
        assert!(fs::symlink_metadata(l.join("home/me/links/dir-link")).is_ok());
        assert_eq!(
            fs::read_to_string(l.join("home/me/target-dir/inside.txt")).unwrap(),
            "inside"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_windows_error_code_and_message_are_kept_not_replaced() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        plain_link_world(&l);
        let s = spaces(&l, &w);
        // The lab: "Could not find a part of the path" (ERROR_PATH_NOT_FOUND = 3).
        let refuse = |_: &Path| -> io::Result<()> { Err(io::Error::from_raw_os_error(3)) };
        let (_, rep, _) = run_with(|run| {
            delete_with_host(
                &s,
                &[lin("/home/me/links/dir-link")],
                true,
                run,
                None,
                &refuse,
            )
        });
        assert_eq!(rep.failed.len(), 1);
        let m = &rep.failed[0].message;
        assert!(m.contains("os error 3"), "{m}");
        assert!(m.contains("In the distribution: not used"), "{m}");
    }

    #[cfg(unix)]
    #[test]
    fn the_distribution_reporting_success_while_the_link_remains_is_not_believed_either() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        plain_link_world(&l);
        let s = spaces(&l, &w);
        let rm = FakeUnlink {
            calls: Default::default(),
            root: l.path().to_path_buf(),
            lock: None,
            succeed: true,
            really_remove: false,
        };
        let nothing = |_: &Path| -> io::Result<()> { Err(io::Error::other("refused")) };
        let (_, rep, _) = run_with(|run| {
            delete_with_host(
                &s,
                &[lin("/home/me/links/dir-link")],
                true,
                run,
                Some(&rm),
                &nothing,
            )
        });
        assert_eq!(rep.failed.len(), 1);
        assert!(rep.failed[0].message.contains("unlink ran without error"));
        assert!(rep.failed[0]
            .message
            .contains("The link is still there: yes"));
    }

    /// REAL WINDOWS test: a directory junction is a reparse-point folder. Deleting it removes the
    /// junction only; the folder it points to keeps its files.
    #[cfg(windows)]
    #[test]
    fn real_windows_junction_is_removed_without_touching_its_target() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        l.file("home/me/target-dir/inside.txt", "inside");
        let link = l.join("home/me/junction");
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(l.join("home/me/target-dir"))
            .output();
        if !matches!(made, Ok(ref o) if o.status.success()) {
            return; // cannot create a junction here
        }
        let s = spaces(&l, &w);
        let (r, rep, _) = run_with(|run| delete(&s, &[lin("/home/me/junction")], true, run));
        r.unwrap();
        assert!(rep.failed.is_empty(), "{:?}", rep.failed);
        assert!(fs::symlink_metadata(&link).is_err());
        assert_eq!(
            fs::read_to_string(l.join("home/me/target-dir/inside.txt")).unwrap(),
            "inside"
        );
    }

    /// REAL WINDOWS test: a folder symlink (when this machine may create one).
    #[cfg(windows)]
    #[test]
    fn real_windows_folder_symlink_is_removed_without_touching_its_target() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        l.file("home/me/target-dir/inside.txt", "inside");
        let link = l.join("home/me/dlink");
        if std::os::windows::fs::symlink_dir(l.join("home/me/target-dir"), &link).is_err() {
            return; // no permission to create symbolic links here
        }
        let s = spaces(&l, &w);
        let (r, rep, _) = run_with(|run| delete(&s, &[lin("/home/me/dlink")], true, run));
        r.unwrap();
        assert!(rep.failed.is_empty(), "{:?}", rep.failed);
        assert!(fs::symlink_metadata(&link).is_err());
        assert_eq!(
            fs::read_to_string(l.join("home/me/target-dir/inside.txt")).unwrap(),
            "inside"
        );
    }

    #[cfg(unix)]
    #[test]
    fn links_are_recognised_from_both_the_entry_itself_and_its_parents_listing() {
        use std::os::unix::fs::symlink;
        let l = TempDir::new("l");
        l.file("d/inside.txt", "x");
        l.file("f.txt", "x");
        symlink(l.join("d"), l.join("dlink")).unwrap();
        symlink(l.join("f.txt"), l.join("flink")).unwrap();
        for (name, is_link) in [
            ("d", false),
            ("f.txt", false),
            ("dlink", true),
            ("flink", true),
        ] {
            let own = fs::symlink_metadata(l.join(name)).unwrap();
            let listed = enumerated_meta(&l.join(name)).expect(name);
            assert_eq!(linkish(&own, false), is_link, "{name} (own)");
            assert_eq!(linkish(&listed, false), is_link, "{name} (listing)");
        }
        assert!(enumerated_meta(&l.join("missing")).is_none());
    }

    #[test]
    fn link_paths_for_the_distribution_come_only_from_inside_its_root() {
        let ctx = DelCtx {
            root: Some(Path::new("/r/root")),
            remover: None,
            host_remove: &remove_link,
        };
        assert_eq!(
            ctx.linux_path(Path::new("/r/root/tmp/x/link")).as_deref(),
            Some("/tmp/x/link")
        );
        assert_eq!(ctx.linux_path(Path::new("/r/root")), None);
        assert_eq!(ctx.linux_path(Path::new("/elsewhere/link")), None);
        let none = DelCtx {
            root: None,
            remover: None,
            host_remove: &remove_link,
        };
        assert_eq!(none.linux_path(Path::new("/r/root/a")), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_permission_error_stops_only_that_item_and_is_explained() {
        use std::os::unix::fs::PermissionsExt;
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        let locked = l.dir("home/me/locked");
        l.file("home/me/locked/inner.txt", "x");
        l.file("home/me/free.txt", "x");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();
        let s = spaces(&l, &w);
        let (r, rep, _) = run_with(|run| {
            delete(
                &s,
                &[lin("/home/me/locked"), lin("/home/me/free.txt")],
                true,
                run,
            )
        });
        let _ = fs::set_permissions(&locked, fs::Permissions::from_mode(0o755));
        r.unwrap();
        assert!(
            !l.join("home/me/free.txt").exists(),
            "the other item was still deleted"
        );
        let root_user = !l.join("home/me/locked").exists();
        if !root_user {
            assert!(
                l.join("home/me/locked/inner.txt").exists(),
                "nothing in the protected folder was removed"
            );
            assert!(
                rep.failed
                    .iter()
                    .any(|f| f.message.contains("permission denied")),
                "{:?}",
                rep.failed
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn creating_in_a_read_only_folder_reports_permission_denied() {
        use std::os::unix::fs::PermissionsExt;
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        let ro = l.dir("home/me/ro");
        fs::set_permissions(&ro, fs::Permissions::from_mode(0o555)).unwrap();
        let s = spaces(&l, &w);
        let r = create_file(&s, &lin("/home/me/ro"), "x.txt");
        fs::set_permissions(&ro, fs::Permissions::from_mode(0o755)).unwrap();
        if let Err(e) = r {
            assert_eq!(e.kind, FsErrorKind::PermissionDenied);
            assert!(e.message.contains("permission denied"));
        }
    }

    #[test]
    fn copy_a_file_and_a_folder_tree() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        l.file("home/me/doc.txt", "hello");
        l.file("home/me/tree/a.txt", "A");
        l.file("home/me/tree/deep/b.txt", "BB");
        l.dir("home/me/dest");
        let s = spaces(&l, &w);
        let (r, rep, _) = run_with(|run| {
            transfer(
                &s,
                &[lin("/home/me/doc.txt"), lin("/home/me/tree")],
                &lin("/home/me/dest"),
                TransferMode::Copy,
                Conflict::KeepBoth,
                run,
            )
        });
        r.unwrap();
        assert_eq!(
            fs::read_to_string(l.join("home/me/dest/doc.txt")).unwrap(),
            "hello"
        );
        assert_eq!(
            fs::read_to_string(l.join("home/me/dest/tree/deep/b.txt")).unwrap(),
            "BB"
        );
        assert!(
            l.join("home/me/doc.txt").exists() && l.join("home/me/tree/a.txt").exists(),
            "copy leaves the originals"
        );
        assert_eq!(rep.items_done, 2);
        assert_eq!(rep.bytes, 8);
        assert_eq!(rep.results.len(), 2);
    }

    #[test]
    fn copy_between_linux_and_a_windows_folder() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        l.file("home/me/report.txt", "R");
        w.file("from-windows.txt", "W");
        let s = spaces(&l, &w);
        let (r, _, _) = run_with(|run| {
            transfer(
                &s,
                &[lin("/home/me/report.txt")],
                &Location::windows("documents", &[]),
                TransferMode::Copy,
                Conflict::KeepBoth,
                run,
            )
        });
        r.unwrap();
        assert_eq!(
            fs::read_to_string(w.join("report.txt")).unwrap(),
            "R",
            "visible on the Windows side at once"
        );
        let (r, _, _) = run_with(|run| {
            transfer(
                &s,
                &[Location::windows("documents", &["from-windows.txt"])],
                &lin("/home/me"),
                TransferMode::Copy,
                Conflict::KeepBoth,
                run,
            )
        });
        r.unwrap();
        assert_eq!(
            fs::read_to_string(l.join("home/me/from-windows.txt")).unwrap(),
            "W"
        );
    }

    #[test]
    fn copy_never_overwrites_it_keeps_both() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        l.file("home/me/a.txt", "source");
        l.file("home/me/dest/a.txt", "precious");
        let s = spaces(&l, &w);
        for _ in 0..2 {
            let (r, _, _) = run_with(|run| {
                transfer(
                    &s,
                    &[lin("/home/me/a.txt")],
                    &lin("/home/me/dest"),
                    TransferMode::Copy,
                    Conflict::KeepBoth,
                    run,
                )
            });
            r.unwrap();
        }
        assert_eq!(
            fs::read_to_string(l.join("home/me/dest/a.txt")).unwrap(),
            "precious"
        );
        assert_eq!(
            fs::read_to_string(l.join("home/me/dest/a (copy).txt")).unwrap(),
            "source"
        );
        assert_eq!(
            fs::read_to_string(l.join("home/me/dest/a (copy 2).txt")).unwrap(),
            "source"
        );
    }

    #[test]
    fn conflict_policies_skip_and_fail_also_never_overwrite() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        l.file("home/me/a.txt", "source");
        l.file("home/me/dest/a.txt", "precious");
        let s = spaces(&l, &w);
        let (r, rep, _) = run_with(|run| {
            transfer(
                &s,
                &[lin("/home/me/a.txt")],
                &lin("/home/me/dest"),
                TransferMode::Copy,
                Conflict::Skip,
                run,
            )
        });
        r.unwrap();
        assert_eq!(rep.skipped.len(), 1);
        assert_eq!(rep.items_done, 0);
        let (r, rep, _) = run_with(|run| {
            transfer(
                &s,
                &[lin("/home/me/a.txt")],
                &lin("/home/me/dest"),
                TransferMode::Move,
                Conflict::Fail,
                run,
            )
        });
        r.unwrap();
        assert_eq!(rep.failed.len(), 1);
        assert_eq!(
            fs::read_to_string(l.join("home/me/dest/a.txt")).unwrap(),
            "precious"
        );
        assert!(
            l.join("home/me/a.txt").exists(),
            "a failed move keeps the original"
        );
    }

    #[test]
    fn copying_a_folder_into_itself_is_refused() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        l.dir("home/me/tree/inner");
        let s = spaces(&l, &w);
        for dest in ["/home/me/tree", "/home/me/tree/inner"] {
            let (r, _, _) = run_with(|run| {
                transfer(
                    &s,
                    &[lin("/home/me/tree")],
                    &lin(dest),
                    TransferMode::Copy,
                    Conflict::KeepBoth,
                    run,
                )
            });
            assert_eq!(r.unwrap_err().kind, FsErrorKind::InvalidPath, "{dest}");
        }
        assert!(!l.join("home/me/tree/tree").exists());
    }

    #[test]
    fn move_renames_in_place_and_across_folders() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        l.file("home/me/a.txt", "A");
        l.dir("home/me/dest");
        let s = spaces(&l, &w);
        let (r, rep, _) = run_with(|run| {
            transfer(
                &s,
                &[lin("/home/me/a.txt")],
                &lin("/home/me/dest"),
                TransferMode::Move,
                Conflict::KeepBoth,
                run,
            )
        });
        r.unwrap();
        assert!(!l.join("home/me/a.txt").exists());
        assert_eq!(
            fs::read_to_string(l.join("home/me/dest/a.txt")).unwrap(),
            "A"
        );
        assert_eq!(rep.results, vec![lin("/home/me/dest/a.txt")]);
        // Moving to where it already is does nothing.
        let (r, rep, _) = run_with(|run| {
            transfer(
                &s,
                &[lin("/home/me/dest/a.txt")],
                &lin("/home/me/dest"),
                TransferMode::Move,
                Conflict::KeepBoth,
                run,
            )
        });
        r.unwrap();
        assert_eq!(rep.skipped.len(), 1);
        assert!(l.join("home/me/dest/a.txt").exists());
    }

    #[test]
    fn move_between_linux_and_windows_folder() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        l.file("home/me/tree/x.txt", "X");
        let s = spaces(&l, &w);
        let (r, _, _) = run_with(|run| {
            transfer(
                &s,
                &[lin("/home/me/tree")],
                &Location::windows("documents", &[]),
                TransferMode::Move,
                Conflict::KeepBoth,
                run,
            )
        });
        r.unwrap();
        assert_eq!(fs::read_to_string(w.join("tree/x.txt")).unwrap(), "X");
        assert!(!l.join("home/me/tree").exists());
    }

    #[test]
    fn a_move_by_copy_removes_the_original_only_after_a_full_copy() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        l.file("src/a.txt", "A");
        l.file("src/sub/b.txt", "B");
        let cancel = CancelToken::new();
        let mut sink = |_: &Progress| {};
        let mut run = Runner::new(&cancel, &mut sink);
        let dst = w.join("out");
        assert!(move_by_copy(&l.join("src"), &dst, &mut run).is_ok());
        assert_eq!(fs::read_to_string(dst.join("sub/b.txt")).unwrap(), "B");
        assert!(!l.join("src").exists());
        let rep = run.finish();
        assert_eq!(rep.files, 2, "the cleanup is not counted as extra work");
    }

    #[cfg(unix)]
    #[test]
    fn links_are_skipped_when_copying_and_keep_the_original_when_moving_by_copy() {
        use std::os::unix::fs::symlink;
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        let outside = TempDir::new("outside");
        outside.file("secret.txt", "s");
        l.file("src/a.txt", "A");
        symlink(outside.path(), l.join("src/link")).unwrap();
        let s = spaces(&l, &w);
        let (r, rep, _) = run_with(|run| {
            transfer(
                &s,
                &[lin("/src")],
                &Location::windows("documents", &[]),
                TransferMode::Copy,
                Conflict::KeepBoth,
                run,
            )
        });
        r.unwrap();
        assert!(w.join("src/a.txt").exists());
        assert!(
            !w.join("src/link").exists() && !w.join("src/link/secret.txt").exists(),
            "links are never followed"
        );
        assert_eq!(rep.skipped.len(), 1);
        // moving by copy with a skipped item must keep the original
        let cancel = CancelToken::new();
        let mut sink = |_: &Progress| {};
        let mut run = Runner::new(&cancel, &mut sink);
        let r = move_by_copy(&l.join("src"), &w.join("moved"), &mut run);
        assert!(matches!(r, Err(Stop::Partial)));
        assert!(l.join("src/a.txt").exists(), "original kept");
        assert_eq!(fs::read_to_string(outside.join("secret.txt")).unwrap(), "s");
    }

    #[test]
    fn a_big_file_is_copied_in_chunks_with_progress() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        let data: Vec<u8> = (0..(3 * CHUNK + 17)).map(|i| (i % 251) as u8).collect();
        fs::create_dir_all(l.join("d")).unwrap();
        fs::write(l.join("d/big.bin"), &data).unwrap();
        let s = spaces(&l, &w);
        let (r, rep, events) = run_with(|run| {
            transfer(
                &s,
                &[lin("/d/big.bin")],
                &Location::windows("documents", &[]),
                TransferMode::Copy,
                Conflict::KeepBoth,
                run,
            )
        });
        r.unwrap();
        assert_eq!(fs::read(w.join("big.bin")).unwrap(), data);
        assert_eq!(rep.bytes, data.len() as u64);
        assert!(events >= 4, "progress per chunk, got {events}");
    }

    #[test]
    fn cancelling_stops_and_removes_the_partial_file() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        let data = vec![7u8; 4 * CHUNK];
        fs::create_dir_all(l.join("d")).unwrap();
        fs::write(l.join("d/big.bin"), &data).unwrap();
        let s = spaces(&l, &w);
        let cancel = CancelToken::new();
        let c2 = cancel.clone();
        let mut sink = move |p: &Progress| {
            if p.bytes >= CHUNK as u64 {
                c2.cancel();
            }
        };
        let mut run = Runner::new(&cancel, &mut sink);
        transfer(
            &s,
            &[lin("/d/big.bin")],
            &Location::windows("documents", &[]),
            TransferMode::Copy,
            Conflict::KeepBoth,
            &mut run,
        )
        .unwrap();
        let rep = run.finish();
        assert!(rep.cancelled);
        assert!(!w.join("big.bin").exists(), "the partial copy is removed");
        assert!(l.join("d/big.bin").exists());
    }

    #[test]
    fn transfer_needs_a_folder_destination_and_sources() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        l.file("a.txt", "a");
        l.file("file", "f");
        let s = spaces(&l, &w);
        let (r, _, _) = run_with(|run| {
            transfer(
                &s,
                &[],
                &lin("/"),
                TransferMode::Copy,
                Conflict::KeepBoth,
                run,
            )
        });
        assert_eq!(r.unwrap_err().kind, FsErrorKind::InvalidPath);
        let (r, _, _) = run_with(|run| {
            transfer(
                &s,
                &[lin("/a.txt")],
                &lin("/file"),
                TransferMode::Copy,
                Conflict::KeepBoth,
                run,
            )
        });
        assert!(r.is_err());
        let (r, _, _) = run_with(|run| {
            transfer(
                &s,
                &[lin("/a.txt")],
                &lin("/missing"),
                TransferMode::Copy,
                Conflict::KeepBoth,
                run,
            )
        });
        assert_eq!(r.unwrap_err().kind, FsErrorKind::NotFound);
    }

    #[test]
    fn copying_many_small_files_stays_correct() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        let dir = l.dir("many");
        for i in 0..1500 {
            fs::write(dir.join(format!("f{i}.txt")), i.to_string()).unwrap();
        }
        let s = spaces(&l, &w);
        let (r, rep, _) = run_with(|run| {
            transfer(
                &s,
                &[lin("/many")],
                &Location::windows("documents", &[]),
                TransferMode::Copy,
                Conflict::KeepBoth,
                run,
            )
        });
        r.unwrap();
        assert_eq!(rep.files, 1500);
        assert_eq!(fs::read_dir(w.join("many")).unwrap().count(), 1500);
        assert_eq!(
            fs::read_to_string(w.join("many/f1234.txt")).unwrap(),
            "1234"
        );
    }
}
