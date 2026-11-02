//! The three kinds of storage the file manager distinguishes, and how a [`Location`] given by the
//! UI becomes a host path.
//!
//! 1. **Linux distribution filesystem** — reached through the distribution's own file server
//!    (`\\wsl.localhost\<distro>\...` on Windows). Paths are Linux paths (`/home/me/Downloads`).
//! 2. **Shared Windows folders** — the user's Known Folders (or chosen replacements). A location is
//!    `(root id, relative components)`: the UI never sends a raw Windows path, so it cannot name
//!    anything outside the configured roots. Containment is re-checked against the real filesystem
//!    (junctions and symlinks cannot lead out of a root).
//! 3. **VLINUX configuration and app data** — never reachable from here at all.
//!
//! Symlinks are never followed blindly: the Linux resolver walks the path one component at a time
//! and translates links that point into a shared Windows folder (`/mnt/c/Users/me/Downloads`) to
//! the Windows location itself; links to other drive mounts are refused.

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::error::{FsError, FsErrorKind, FsResult};
use super::path::{check_host_component, linux_components, linux_join};

const MAX_FOLLOW: usize = 40;

/// A place in one of the two browsable spaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "space", rename_all = "snake_case")]
pub enum Location {
    Linux { path: String },
    Windows { root: String, rel: Vec<String> },
}

impl Location {
    pub fn linux(path: &str) -> Self {
        Location::Linux {
            path: path.to_string(),
        }
    }
    pub fn windows(root: &str, rel: &[&str]) -> Self {
        Location::Windows {
            root: root.to_string(),
            rel: rel.iter().map(|s| s.to_string()).collect(),
        }
    }
}

/// One shared Windows folder the file manager may browse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowsRoot {
    pub id: String,
    pub label: String,
    /// Real Windows directory (host path).
    pub dir: PathBuf,
    /// How the distribution sees it (`/mnt/d/Media/Downloads`), if it is reachable that way.
    pub linux_mount: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Spaces {
    /// Host path of the distribution's root (`\\wsl.localhost\Debian`); `None` = no distribution.
    pub linux_root: Option<PathBuf>,
    pub linux_home: Option<String>,
    /// The distribution's automount root (`/mnt`). Drive mounts below it are never browsed through
    /// the Linux tree; the shared folders section is the way to Windows files.
    pub mount_root: String,
    pub windows: Vec<WindowsRoot>,
    /// Links created by VLINUX: `(linux path of the link, root id)`. Recorded mappings are trusted
    /// for navigation so the file manager does not depend on how the file server presents links.
    pub mapped_links: Vec<(String, String)>,
}

/// Resolved entry (not followed if it is a link).
#[derive(Debug, Clone)]
pub struct EntryRef {
    pub location: Location,
    pub parent: Location,
    pub name: String,
    pub host: PathBuf,
}

fn comps_eq(a: &[String], b: &[String]) -> bool {
    a == b
}

fn is_drive_like(name: &str) -> bool {
    name.len() == 1 && name.chars().all(|c| c.is_ascii_alphabetic())
}

impl Spaces {
    pub fn root(&self, id: &str) -> Option<&WindowsRoot> {
        self.windows.iter().find(|r| r.id == id)
    }

    fn no_distro() -> FsError {
        FsError::new(
            FsErrorKind::Unavailable,
            "No Linux distribution is selected or reachable. Choose one in Linux Setup, or browse the Windows shared folders.",
        )
    }

    fn linux_host(&self, comps: &[String]) -> FsResult<PathBuf> {
        let root = self.linux_root.as_ref().ok_or_else(Self::no_distro)?;
        let mut p = root.clone();
        for c in comps {
            check_host_component(c)?;
            p.push(c);
        }
        Ok(p)
    }

    /// Is `cur` (a *resolved* Linux path) at or below a configured shared folder's mount path?
    /// Returns the root and the number of leading components the mount covers.
    fn mount_match(&self, cur: &[String]) -> Option<(&WindowsRoot, usize)> {
        for r in &self.windows {
            if let Some(m) = &r.linux_mount {
                if let Ok(mc) = linux_components(m) {
                    if !mc.is_empty() && cur.len() >= mc.len() && comps_eq(&cur[..mc.len()], &mc) {
                        return Some((r, mc.len()));
                    }
                }
            }
        }
        None
    }

    fn link_match(&self, cur: &[String]) -> Option<(&str, usize)> {
        for (link, root) in &self.mapped_links {
            if let Ok(lc) = linux_components(link) {
                if !lc.is_empty() && cur.len() >= lc.len() && comps_eq(&cur[..lc.len()], &lc) {
                    return Some((root.as_str(), lc.len()));
                }
            }
        }
        None
    }

    /// `cur` is a proper prefix of a shared folder's mount path (`/mnt/d` on the way to
    /// `/mnt/d/Media/Downloads`): passing through it is fine, the mount itself decides later.
    fn on_the_way_to_a_mount(&self, cur: &[String]) -> bool {
        self.windows.iter().filter_map(|r| r.linux_mount.as_deref()).any(|m| {
            matches!(linux_components(m), Ok(mc) if mc.len() > cur.len() && comps_eq(&mc[..cur.len()], cur))
        })
    }

    fn blocked_drive(&self, cur: &[String]) -> bool {
        match linux_components(&self.mount_root) {
            Ok(mr) if !mr.is_empty() => {
                cur.len() > mr.len()
                    && comps_eq(&cur[..mr.len()], &mr)
                    && is_drive_like(&cur[mr.len()])
            }
            _ => false,
        }
    }

    /// Apply remaining components (which may still contain `.`/`..` from a link target) to a
    /// Windows-relative path. Leaving the root is an error.
    fn windows_with_rest(
        &self,
        root: &str,
        mut rel: Vec<String>,
        rest: &VecDeque<String>,
    ) -> FsResult<Location> {
        for c in rest {
            match c.as_str() {
                "" | "." => {}
                ".." => {
                    if rel.pop().is_none() {
                        return Err(FsError::new(
                            FsErrorKind::OutsideRoot,
                            "That link leads out of the shared Windows folder.",
                        ));
                    }
                }
                other => {
                    check_host_component(other)?;
                    rel.push(other.to_string());
                }
            }
        }
        Ok(Location::Windows {
            root: root.to_string(),
            rel,
        })
    }

    /// Resolve a Linux path by walking its components and following links one by one.
    pub fn resolve_linux(&self, path: &str) -> FsResult<Location> {
        let comps = linux_components(path)?;
        let mut cur: Vec<String> = Vec::new();
        let mut queue: VecDeque<String> = comps.into();
        let mut follows = 0usize;
        // Once a component does not exist, nothing below it can be a link: stop asking the
        // filesystem, but keep applying the mount and shared-folder rules.
        let mut missing = false;
        while let Some(c) = queue.pop_front() {
            match c.as_str() {
                "" | "." => continue,
                ".." => {
                    cur.pop();
                    continue;
                }
                _ => {}
            }
            cur.push(c);

            if let Some((root, n)) = self.link_match(&cur) {
                let rel: Vec<String> = cur[n..].to_vec();
                return self.windows_with_rest(root, rel, &queue);
            }
            if let Some((root, n)) = self.mount_match(&cur) {
                let rel: Vec<String> = cur[n..].to_vec();
                return self.windows_with_rest(&root.id.clone(), rel, &queue);
            }
            if self.blocked_drive(&cur) && !self.on_the_way_to_a_mount(&cur) {
                return Err(FsError::new(
                    FsErrorKind::OutsideRoot,
                    format!(
                        "{} is a Windows drive mounted into Linux. Windows files are reached through the shared folders in the sidebar, not through the Linux tree.",
                        linux_join(&cur)
                    ),
                ));
            }

            if missing {
                continue;
            }
            let host = self.linux_host(&cur)?;
            match fs::symlink_metadata(&host) {
                Ok(m) if m.file_type().is_symlink() => {
                    follows += 1;
                    if follows > MAX_FOLLOW {
                        return Err(FsError::new(
                            FsErrorKind::Refused,
                            "Too many levels of symbolic links (possible loop).",
                        ));
                    }
                    let target = fs::read_link(&host)
                        .map_err(|e| FsError::from_io(&e, "read the link", &linux_join(&cur)))?
                        .to_string_lossy()
                        .replace('\\', "/");
                    cur.pop();
                    if target.starts_with('/') {
                        cur.clear();
                    }
                    let mut pushed: Vec<String> = target
                        .split('/')
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_string())
                        .collect();
                    while let Some(t) = pushed.pop() {
                        queue.push_front(t);
                    }
                }
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => missing = true,
                Err(e) => return Err(FsError::from_io(&e, "open", &linux_join(&cur))),
            }
        }
        if self.blocked_drive(&cur) {
            // Ended on the way to a shared mount: the folders around it are not shared.
            return Err(FsError::new(
                FsErrorKind::OutsideRoot,
                format!(
                    "{} is part of a Windows drive. Use the shared folders in the sidebar instead.",
                    linux_join(&cur)
                ),
            ));
        }
        Ok(Location::Linux {
            path: linux_join(&cur),
        })
    }

    /// Resolve any location to its final form (links followed).
    pub fn resolve(&self, loc: &Location) -> FsResult<Location> {
        match loc {
            Location::Linux { path } => self.resolve_linux(path),
            Location::Windows { root, rel } => {
                self.root(root).ok_or_else(|| {
                    FsError::new(FsErrorKind::OutsideRoot, "Unknown shared folder.")
                })?;
                for c in rel {
                    check_host_component(c)?;
                }
                Ok(Location::Windows {
                    root: root.clone(),
                    rel: rel.clone(),
                })
            }
        }
    }

    /// Host path of a *resolved* location. `follow_last = false` is for operating on a link entry
    /// itself (rename/delete a link) rather than what it points to.
    pub fn host_of_resolved(&self, loc: &Location, follow_last: bool) -> FsResult<PathBuf> {
        match loc {
            Location::Linux { path } => self.linux_host(&linux_components(path)?),
            Location::Windows { root, rel } => {
                let r = self.root(root).ok_or_else(|| {
                    FsError::new(FsErrorKind::OutsideRoot, "Unknown shared folder.")
                })?;
                let mut p = r.dir.clone();
                for c in rel {
                    check_host_component(c)?;
                    p.push(c);
                }
                ensure_inside(&r.dir, &p, follow_last)?;
                Ok(p)
            }
        }
    }

    /// A directory to read or write inside: links are followed (and checked).
    pub fn resolve_dir(&self, loc: &Location) -> FsResult<(Location, PathBuf)> {
        let resolved = self.resolve(loc)?;
        let host = self.host_of_resolved(&resolved, true)?;
        Ok((resolved, host))
    }

    /// A single entry: its parent is resolved (links followed), the entry itself is not.
    pub fn resolve_entry(&self, loc: &Location) -> FsResult<EntryRef> {
        match loc {
            Location::Linux { path } => {
                let comps = linux_components(path)?;
                let (name, parent_comps) = match comps.split_last() {
                    Some((n, rest)) => (n.clone(), rest.to_vec()),
                    None => {
                        return Err(FsError::new(
                            FsErrorKind::Refused,
                            "The root folder cannot be changed.",
                        ))
                    }
                };
                let parent = self.resolve_linux(&linux_join(&parent_comps))?;
                self.entry_in(parent, name)
            }
            Location::Windows { root, rel } => {
                let (name, parent_rel) = match rel.split_last() {
                    Some((n, rest)) => (n.clone(), rest.to_vec()),
                    None => {
                        return Err(FsError::new(
                            FsErrorKind::Refused,
                            "A shared folder itself cannot be changed here.",
                        ))
                    }
                };
                let parent = self.resolve(&Location::Windows {
                    root: root.clone(),
                    rel: parent_rel,
                })?;
                self.entry_in(parent, name)
            }
        }
    }

    /// The entry called `name` in the (resolved) directory `parent`.
    pub fn entry_in(&self, parent: Location, name: String) -> FsResult<EntryRef> {
        check_host_component(&name)?;
        let location = match &parent {
            Location::Linux { path } => {
                let mut c = linux_components(path)?;
                c.push(name.clone());
                Location::Linux {
                    path: linux_join(&c),
                }
            }
            Location::Windows { root, rel } => {
                let mut r = rel.clone();
                r.push(name.clone());
                Location::Windows {
                    root: root.clone(),
                    rel: r,
                }
            }
        };
        // Containment is checked on the parent (the entry may itself be a link).
        let host = self.host_of_resolved(&location, false)?;
        Ok(EntryRef {
            location,
            parent,
            name,
            host,
        })
    }
}

/// `full` must stay inside `root` after resolving links/junctions. When `follow_last` is false
/// the final component is not resolved (it may be a link we are about to remove or rename).
pub fn ensure_inside(root: &Path, full: &Path, follow_last: bool) -> FsResult<()> {
    let canon_root = fs::canonicalize(root)
        .map_err(|e| FsError::from_io(&e, "open", &root.display().to_string()))?;
    let mut probe: PathBuf = if follow_last {
        full.to_path_buf()
    } else {
        full.parent().unwrap_or(full).to_path_buf()
    };
    loop {
        match fs::canonicalize(&probe) {
            Ok(c) => {
                return if c.starts_with(&canon_root) {
                    Ok(())
                } else {
                    Err(FsError::new(FsErrorKind::OutsideRoot, "That location leads out of the shared folder (through a link or junction), so it was not opened."))
                };
            }
            Err(_) => {
                if !probe.pop() {
                    return Err(FsError::new(
                        FsErrorKind::NotFound,
                        "That location does not exist.",
                    ));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::testutil::TempDir;
    use super::*;

    fn spaces(linux: &TempDir, win: &TempDir) -> Spaces {
        Spaces {
            linux_root: Some(linux.path().to_path_buf()),
            linux_home: Some("/home/me".into()),
            mount_root: "/mnt".into(),
            windows: vec![WindowsRoot {
                id: "downloads".into(),
                label: "Downloads".into(),
                dir: win.path().to_path_buf(),
                linux_mount: Some("/mnt/d/Media/Downloads".into()),
            }],
            mapped_links: vec![],
        }
    }

    #[test]
    fn plain_linux_paths_resolve_to_themselves() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        l.dir("home/me/docs");
        let s = spaces(&l, &w);
        assert_eq!(
            s.resolve_linux("/home/me/docs").unwrap(),
            Location::linux("/home/me/docs")
        );
        // A path that does not exist yet is still a valid location (for create).
        assert_eq!(
            s.resolve_linux("/home/me/new/dir").unwrap(),
            Location::linux("/home/me/new/dir")
        );
        assert_eq!(s.resolve_linux("/").unwrap(), Location::linux("/"));
    }

    #[test]
    fn dotdot_and_relative_paths_never_get_through() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        let s = spaces(&l, &w);
        for p in ["/home/../etc", "home/me", "/home/me/..", "/a\0"] {
            assert_eq!(
                s.resolve_linux(p).unwrap_err().kind,
                FsErrorKind::InvalidPath,
                "{p:?}"
            );
        }
    }

    #[test]
    fn no_distribution_means_unavailable() {
        let s = Spaces::default();
        assert_eq!(
            s.resolve_linux("/home").map(|_| ()).unwrap_err().kind,
            FsErrorKind::Unavailable
        );
        assert_eq!(
            s.resolve_dir(&Location::linux("/home"))
                .map(|_| ())
                .unwrap_err()
                .kind,
            FsErrorKind::Unavailable
        );
    }

    #[test]
    fn windows_locations_need_a_known_root() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        w.dir("sub");
        let s = spaces(&l, &w);
        let (loc, host) = s
            .resolve_dir(&Location::windows("downloads", &["sub"]))
            .unwrap();
        assert_eq!(loc, Location::windows("downloads", &["sub"]));
        assert_eq!(host, w.path().join("sub"));
        assert_eq!(
            s.resolve_dir(&Location::windows("system32", &[]))
                .map(|_| ())
                .unwrap_err()
                .kind,
            FsErrorKind::OutsideRoot
        );
    }

    #[test]
    fn windows_relative_parts_cannot_traverse_or_address_streams() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        let s = spaces(&l, &w);
        for rel in [
            vec![".."],
            vec!["a", ".."],
            vec!["C:"],
            vec!["x/y"],
            vec!["a\\b"],
            vec!["file:stream"],
            vec![""],
            vec!["*"],
        ] {
            let loc = Location::Windows {
                root: "downloads".into(),
                rel: rel.iter().map(|s| s.to_string()).collect(),
            };
            assert!(s.resolve_dir(&loc).is_err(), "{rel:?}");
        }
    }

    #[test]
    fn drive_mounts_in_the_linux_tree_are_blocked() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        l.dir("mnt/c/Windows");
        let s = spaces(&l, &w);
        assert_eq!(
            s.resolve_linux("/mnt/c").unwrap_err().kind,
            FsErrorKind::OutsideRoot
        );
        assert_eq!(
            s.resolve_linux("/mnt/c/Windows/System32").unwrap_err().kind,
            FsErrorKind::OutsideRoot
        );
        // /mnt itself and non-drive names are ordinary directories.
        assert!(s.resolve_linux("/mnt").is_ok());
        assert!(s.resolve_linux("/mnt/wsl").is_ok());
    }

    #[test]
    fn a_path_inside_a_shared_mount_becomes_the_windows_location() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        let s = spaces(&l, &w);
        assert_eq!(
            s.resolve_linux("/mnt/d/Media/Downloads").unwrap(),
            Location::windows("downloads", &[])
        );
        assert_eq!(
            s.resolve_linux("/mnt/d/Media/Downloads/Sub/file.txt")
                .unwrap(),
            Location::windows("downloads", &["Sub", "file.txt"])
        );
        // Another folder on the same drive is NOT reachable, nor are the folders leading to the mount.
        assert_eq!(
            s.resolve_linux("/mnt/d/Media/Secret").unwrap_err().kind,
            FsErrorKind::OutsideRoot
        );
        assert_eq!(
            s.resolve_linux("/mnt/d/Media").unwrap_err().kind,
            FsErrorKind::OutsideRoot
        );
        assert_eq!(
            s.resolve_linux("/mnt/d").unwrap_err().kind,
            FsErrorKind::OutsideRoot
        );
    }

    #[test]
    fn recorded_mappings_navigate_without_touching_the_filesystem() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        let mut s = spaces(&l, &w);
        s.mapped_links
            .push(("/home/me/Downloads".into(), "downloads".into()));
        // `/home/me/Downloads` does not even exist in the fake distribution root.
        assert_eq!(
            s.resolve_linux("/home/me/Downloads").unwrap(),
            Location::windows("downloads", &[])
        );
        assert_eq!(
            s.resolve_linux("/home/me/Downloads/a/b").unwrap(),
            Location::windows("downloads", &["a", "b"])
        );
        assert_eq!(
            s.resolve_linux("/home/me/Documents").unwrap(),
            Location::linux("/home/me/Documents")
        );
    }

    #[cfg(unix)]
    mod unix {
        use super::*;
        use std::os::unix::fs::symlink;

        #[test]
        fn links_inside_the_distribution_are_followed_component_wise() {
            let l = TempDir::new("l");
            let w = TempDir::new("w");
            l.dir("home/me/real/inner");
            symlink("real", l.join("home/me/rel")).unwrap();
            symlink("/home/me/real", l.join("home/me/abs")).unwrap();
            let s = spaces(&l, &w);
            assert_eq!(
                s.resolve_linux("/home/me/rel/inner").unwrap(),
                Location::linux("/home/me/real/inner")
            );
            assert_eq!(
                s.resolve_linux("/home/me/abs").unwrap(),
                Location::linux("/home/me/real")
            );
        }

        #[test]
        fn a_link_into_a_shared_mount_becomes_the_windows_location() {
            let l = TempDir::new("l");
            let w = TempDir::new("w");
            l.dir("home/me");
            symlink("/mnt/d/Media/Downloads", l.join("home/me/Downloads")).unwrap();
            let s = spaces(&l, &w);
            assert_eq!(
                s.resolve_linux("/home/me/Downloads").unwrap(),
                Location::windows("downloads", &[])
            );
            assert_eq!(
                s.resolve_linux("/home/me/Downloads/x").unwrap(),
                Location::windows("downloads", &["x"])
            );
        }

        #[test]
        fn a_link_to_an_unshared_drive_location_is_refused() {
            let l = TempDir::new("l");
            let w = TempDir::new("w");
            l.dir("home/me");
            symlink("/mnt/c/Users", l.join("home/me/users")).unwrap();
            let s = spaces(&l, &w);
            assert_eq!(
                s.resolve_linux("/home/me/users").unwrap_err().kind,
                FsErrorKind::OutsideRoot
            );
        }

        #[test]
        fn link_loops_are_detected() {
            let l = TempDir::new("l");
            let w = TempDir::new("w");
            l.dir("home");
            symlink("/home/b", l.join("home/a")).unwrap();
            symlink("/home/a", l.join("home/b")).unwrap();
            let s = spaces(&l, &w);
            assert_eq!(
                s.resolve_linux("/home/a").unwrap_err().kind,
                FsErrorKind::Refused
            );
        }

        #[test]
        fn dotdot_inside_a_link_target_stays_within_the_resolved_path() {
            let l = TempDir::new("l");
            let w = TempDir::new("w");
            l.dir("home/me/a/b");
            l.dir("home/me/c");
            symlink("../../c", l.join("home/me/a/b/up")).unwrap();
            let s = spaces(&l, &w);
            assert_eq!(
                s.resolve_linux("/home/me/a/b/up").unwrap(),
                Location::linux("/home/me/c")
            );
        }

        #[test]
        fn a_junction_or_link_inside_a_windows_root_cannot_lead_out() {
            let l = TempDir::new("l");
            let w = TempDir::new("w");
            let outside = TempDir::new("outside");
            outside.file("secret.txt", "s");
            symlink(outside.path(), w.join("escape")).unwrap();
            let s = spaces(&l, &w);
            let e = s
                .resolve_dir(&Location::windows("downloads", &["escape"]))
                .map(|_| ())
                .unwrap_err();
            assert_eq!(e.kind, FsErrorKind::OutsideRoot);
            let e = s
                .resolve_dir(&Location::windows("downloads", &["escape", "x"]))
                .map(|_| ())
                .unwrap_err();
            assert_eq!(e.kind, FsErrorKind::OutsideRoot);
            // The link itself, as an entry, can still be addressed (to delete or rename it).
            assert!(s
                .resolve_entry(&Location::windows("downloads", &["escape"]))
                .is_ok());
        }
    }
}
