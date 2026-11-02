//! Directory enumeration with incremental paging.
//!
//! Opening a folder reads the *names and types* once (cheap, even for huge folders), sorts them
//! and remembers them in a small server-side session. The UI then asks for pages; file sizes and
//! dates (one `stat` each, which is slow across the WSL/Windows boundary) are only fetched for the
//! entries of the page being shown. Nothing is polled and nothing is watched.

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use super::error::{FsError, FsErrorKind, FsResult};
use super::filetype::{classify_file, open_policy, FileClass, OpenPolicy, FOLDER, LINK, OTHER};
use super::path::{linux_components, linux_join};
use super::space::{Location, Spaces};

pub const DEFAULT_PAGE: usize = 200;
pub const MAX_PAGE: usize = 1000;
/// Entries kept per folder; beyond this the listing says it was truncated.
pub const MAX_ENTRIES: usize = 500_000;
const MAX_SESSIONS: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SortKey {
    #[default]
    Name,
    Size,
    Modified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ListOptions {
    pub show_hidden: bool,
    pub sort: SortKey,
    pub descending: bool,
}

impl Default for ListOptions {
    fn default() -> Self {
        Self { show_hidden: false, sort: SortKey::Name, descending: false }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Entry {
    pub name: String,
    pub kind: EntryKind,
    pub size: Option<u64>,
    /// Seconds since the Unix epoch.
    pub modified: Option<u64>,
    pub hidden: bool,
    pub readonly: bool,
    pub class: FileClass,
    pub open: OpenPolicy,
    /// For links: where they point (as the file server reports it).
    pub link_target: Option<String>,
    /// For links: `Some(true)` if the target is a folder, `Some(false)` if not, `None` if unknown.
    pub link_is_dir: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Page {
    pub offset: usize,
    pub total: usize,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Crumb {
    pub label: String,
    pub location: Location,
}

#[derive(Debug, Clone, Serialize)]
pub struct ListingHead {
    pub id: String,
    /// What was asked for (normalised).
    pub location: Location,
    /// Where it really is after links were followed. Differs from `location` when, for example,
    /// `~/Downloads` is a shared Windows folder.
    pub resolved: Location,
    pub crumbs: Vec<Crumb>,
    pub parent: Option<Location>,
    pub total: usize,
    pub truncated: bool,
    /// Names that could not be shown (not valid text).
    pub unreadable: usize,
    pub page: Page,
}

struct Raw {
    name: String,
    kind: EntryKind,
    hidden: bool,
    size: Option<u64>,
    modified: Option<u64>,
}

struct Session {
    host: PathBuf,
    entries: Vec<Raw>,
}

#[derive(Default)]
pub struct ListStore {
    sessions: Mutex<(HashMap<String, Session>, VecDeque<String>)>,
    next: AtomicU64,
}

fn secs(t: std::io::Result<std::time::SystemTime>) -> Option<u64> {
    t.ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs())
}

#[cfg(windows)]
fn windows_hidden(m: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN: u32 = 0x2;
    m.file_attributes() & HIDDEN != 0
}
#[cfg(not(windows))]
fn windows_hidden(_m: &fs::Metadata) -> bool {
    false
}

fn kind_of(ft: &fs::FileType) -> EntryKind {
    if ft.is_symlink() {
        EntryKind::Symlink
    } else if ft.is_dir() {
        EntryKind::Dir
    } else if ft.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    }
}

fn describe(host_dir: &Path, raw: &Raw) -> Entry {
    let path = host_dir.join(&raw.name);
    let meta = fs::symlink_metadata(&path).ok();
    let (size, modified, readonly) = match &meta {
        Some(m) => (if m.is_file() { Some(m.len()) } else { None }, secs(m.modified()), m.permissions().readonly()),
        None => (raw.size, raw.modified, false),
    };
    let (link_target, link_is_dir) = if raw.kind == EntryKind::Symlink {
        let target = fs::read_link(&path).ok().map(|t| t.to_string_lossy().replace('\\', "/"));
        let is_dir = fs::metadata(&path).ok().map(|m| m.is_dir());
        (target, is_dir)
    } else {
        (None, None)
    };
    let class = match raw.kind {
        EntryKind::Dir => FOLDER,
        EntryKind::Symlink => {
            if link_is_dir == Some(true) {
                FOLDER
            } else {
                LINK
            }
        }
        EntryKind::Other => OTHER,
        EntryKind::File => classify_file(&raw.name),
    };
    Entry {
        name: raw.name.clone(),
        kind: raw.kind,
        size,
        modified,
        hidden: raw.hidden,
        readonly,
        class,
        open: if raw.kind == EntryKind::File { open_policy(&raw.name) } else { OpenPolicy { allowed: false, reason: None } },
        link_target,
        link_is_dir,
    }
}

fn crumbs_for(resolved_request: &Location, spaces: &Spaces) -> (Vec<Crumb>, Option<Location>) {
    match resolved_request {
        Location::Linux { path } => {
            let comps = linux_components(path).unwrap_or_default();
            let mut crumbs = vec![Crumb { label: "/".into(), location: Location::Linux { path: "/".into() } }];
            for i in 0..comps.len() {
                crumbs.push(Crumb { label: comps[i].clone(), location: Location::Linux { path: linux_join(&comps[..=i]) } });
            }
            let parent = if comps.is_empty() { None } else { Some(Location::Linux { path: linux_join(&comps[..comps.len() - 1]) }) };
            (crumbs, parent)
        }
        Location::Windows { root, rel } => {
            let label = spaces.root(root).map(|r| r.label.clone()).unwrap_or_else(|| root.clone());
            let mut crumbs = vec![Crumb { label, location: Location::Windows { root: root.clone(), rel: vec![] } }];
            for i in 0..rel.len() {
                crumbs.push(Crumb { label: rel[i].clone(), location: Location::Windows { root: root.clone(), rel: rel[..=i].to_vec() } });
            }
            let parent = if rel.is_empty() { None } else { Some(Location::Windows { root: root.clone(), rel: rel[..rel.len() - 1].to_vec() }) };
            (crumbs, parent)
        }
    }
}

fn name_key(s: &str) -> String {
    s.to_lowercase()
}

impl ListStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read `location`, sort, remember the listing and return its first page.
    pub fn open_dir(&self, spaces: &Spaces, location: &Location, opts: &ListOptions, page_size: usize) -> FsResult<ListingHead> {
        let page_size = page_size.clamp(1, MAX_PAGE);
        // Normalise what was asked for (this is what crumbs and "up" are built from).
        let requested = match location {
            Location::Linux { path } => Location::Linux { path: linux_join(&linux_components(path)?) },
            other => other.clone(),
        };
        let (resolved, host) = spaces.resolve_dir(&requested)?;
        let name_for_errors = display_name(&requested);

        let rd = fs::read_dir(&host).map_err(|e| FsError::from_io(&e, "open", &name_for_errors))?;
        let need_meta = opts.sort != SortKey::Name;
        let mut raws: Vec<Raw> = Vec::new();
        let mut unreadable = 0usize;
        let mut truncated = false;
        for item in rd {
            let Ok(item) = item else {
                unreadable += 1;
                continue;
            };
            let os_name = item.file_name();
            let Some(name) = os_name.to_str().map(|s| s.to_string()) else {
                unreadable += 1;
                continue;
            };
            let Ok(ft) = item.file_type() else {
                unreadable += 1;
                continue;
            };
            let mut hidden = name.starts_with('.');
            let mut size = None;
            let mut modified = None;
            if need_meta || cfg!(windows) {
                if let Ok(m) = item.metadata() {
                    hidden = hidden || windows_hidden(&m);
                    if need_meta {
                        size = if m.is_file() { Some(m.len()) } else { None };
                        modified = secs(m.modified());
                    }
                }
            }
            if hidden && !opts.show_hidden {
                continue;
            }
            if raws.len() >= MAX_ENTRIES {
                truncated = true;
                break;
            }
            raws.push(Raw { name, kind: kind_of(&ft), hidden, size, modified });
        }

        raws.sort_by(|a, b| {
            let dirs_first = (b.kind == EntryKind::Dir).cmp(&(a.kind == EntryKind::Dir));
            let primary = match opts.sort {
                SortKey::Name => name_key(&a.name).cmp(&name_key(&b.name)),
                SortKey::Size => a.size.unwrap_or(0).cmp(&b.size.unwrap_or(0)),
                SortKey::Modified => a.modified.unwrap_or(0).cmp(&b.modified.unwrap_or(0)),
            };
            let primary = if opts.descending { primary.reverse() } else { primary };
            dirs_first.then(primary).then_with(|| a.name.cmp(&b.name))
        });

        let total = raws.len();
        let first: Vec<Entry> = raws.iter().take(page_size).map(|r| describe(&host, r)).collect();
        let id = format!("l{}", self.next.fetch_add(1, Ordering::SeqCst) + 1);
        {
            let mut g = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
            g.0.insert(id.clone(), Session { host: host.clone(), entries: raws });
            g.1.push_back(id.clone());
            while g.1.len() > MAX_SESSIONS {
                if let Some(old) = g.1.pop_front() {
                    g.0.remove(&old);
                }
            }
        }
        let (crumbs, parent) = crumbs_for(&requested, spaces);
        Ok(ListingHead {
            id,
            location: requested,
            resolved,
            crumbs,
            parent,
            total,
            truncated,
            unreadable,
            page: Page { offset: 0, total, entries: first },
        })
    }

    pub fn page(&self, id: &str, offset: usize, limit: usize) -> FsResult<Page> {
        let limit = limit.clamp(1, MAX_PAGE);
        // Describe outside the lock: each entry costs a `stat`.
        let (host, total, slice) = {
            let g = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
            let s = g
                .0
                .get(id)
                .ok_or_else(|| FsError::new(FsErrorKind::NotFound, "That folder view has expired. Refresh to load it again."))?;
            let slice: Vec<Raw> = s
                .entries
                .iter()
                .skip(offset)
                .take(limit)
                .map(|r| Raw { name: r.name.clone(), kind: r.kind, hidden: r.hidden, size: r.size, modified: r.modified })
                .collect();
            (s.host.clone(), s.entries.len(), slice)
        };
        let entries = slice.iter().map(|r| describe(&host, r)).collect();
        Ok(Page { offset, total, entries })
    }

    pub fn close(&self, id: &str) {
        let mut g = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
        g.0.remove(id);
        g.1.retain(|x| x != id);
    }

    pub fn open_sessions(&self) -> usize {
        self.sessions.lock().unwrap_or_else(|p| p.into_inner()).0.len()
    }
}

fn display_name(loc: &Location) -> String {
    match loc {
        Location::Linux { path } => path.clone(),
        Location::Windows { root, rel } => {
            if rel.is_empty() {
                root.clone()
            } else {
                rel.join("/")
            }
        }
    }
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
            windows: vec![WindowsRoot { id: "documents".into(), label: "Documents".into(), dir: win.path().to_path_buf(), linux_mount: None }],
            mapped_links: vec![],
        }
    }

    fn names(p: &Page) -> Vec<&str> {
        p.entries.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn lists_a_linux_directory_folders_first_then_by_name() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        l.file("home/me/b.txt", "bb");
        l.file("home/me/A.md", "a");
        l.dir("home/me/zdir");
        l.dir("home/me/adir");
        let store = ListStore::new();
        let head = store.open_dir(&spaces(&l, &w), &Location::linux("/home/me"), &ListOptions::default(), 50).unwrap();
        assert_eq!(names(&head.page), ["adir", "zdir", "A.md", "b.txt"]);
        assert_eq!(head.total, 4);
        let file = head.page.entries.iter().find(|e| e.name == "b.txt").unwrap();
        assert_eq!(file.size, Some(2));
        assert!(file.modified.is_some());
        assert_eq!(file.class.id, "text");
        assert!(file.open.allowed);
        assert_eq!(head.page.entries[0].kind, EntryKind::Dir);
        assert_eq!(head.page.entries[0].size, None);
    }

    #[test]
    fn crumbs_and_parent_follow_the_requested_path() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        l.dir("home/me/docs");
        let store = ListStore::new();
        let head = store.open_dir(&spaces(&l, &w), &Location::linux("//home/me/./docs/"), &ListOptions::default(), 10).unwrap();
        assert_eq!(head.location, Location::linux("/home/me/docs"));
        let labels: Vec<&str> = head.crumbs.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, ["/", "home", "me", "docs"]);
        assert_eq!(head.parent, Some(Location::linux("/home/me")));
        let root = store.open_dir(&spaces(&l, &w), &Location::linux("/"), &ListOptions::default(), 10).unwrap();
        assert_eq!(root.parent, None);
    }

    #[test]
    fn hidden_files_follow_the_toggle() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        l.file("home/me/.bashrc", "");
        l.file("home/me/visible", "");
        let store = ListStore::new();
        let s = spaces(&l, &w);
        let hidden_off = store.open_dir(&s, &Location::linux("/home/me"), &ListOptions::default(), 10).unwrap();
        assert_eq!(names(&hidden_off.page), ["visible"]);
        let on = ListOptions { show_hidden: true, ..ListOptions::default() };
        let hidden_on = store.open_dir(&s, &Location::linux("/home/me"), &on, 10).unwrap();
        assert_eq!(names(&hidden_on.page), [".bashrc", "visible"]);
        assert!(hidden_on.page.entries[0].hidden);
    }

    #[test]
    fn sorting_by_size_and_descending() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        l.file("d/small", "1");
        l.file("d/big", "123456");
        l.file("d/mid", "123");
        let store = ListStore::new();
        let s = spaces(&l, &w);
        let asc = ListOptions { sort: SortKey::Size, ..ListOptions::default() };
        assert_eq!(names(&store.open_dir(&s, &Location::linux("/d"), &asc, 10).unwrap().page), ["small", "mid", "big"]);
        let desc = ListOptions { sort: SortKey::Size, descending: true, ..ListOptions::default() };
        assert_eq!(names(&store.open_dir(&s, &Location::linux("/d"), &desc, 10).unwrap().page), ["big", "mid", "small"]);
    }

    #[test]
    fn a_large_directory_is_paged_not_loaded_at_once() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        let dir = l.dir("big");
        for i in 0..3000 {
            fs::write(dir.join(format!("file-{i:05}.txt")), "x").unwrap();
        }
        let store = ListStore::new();
        let s = spaces(&l, &w);
        let head = store.open_dir(&s, &Location::linux("/big"), &ListOptions::default(), 200).unwrap();
        assert_eq!(head.total, 3000);
        assert_eq!(head.page.entries.len(), 200, "only the first page is described");
        assert_eq!(head.page.entries[0].name, "file-00000.txt");
        let p2 = store.page(&head.id, 200, 200).unwrap();
        assert_eq!(p2.entries.len(), 200);
        assert_eq!(p2.entries[0].name, "file-00200.txt");
        let last = store.page(&head.id, 2900, 1000).unwrap();
        assert_eq!(last.entries.len(), 100);
        assert_eq!(last.entries.last().unwrap().name, "file-02999.txt");
        assert!(store.page(&head.id, 5000, 10).unwrap().entries.is_empty());
        // limits are clamped
        assert_eq!(store.page(&head.id, 0, 100_000).unwrap().entries.len(), MAX_PAGE);
    }

    #[test]
    fn old_sessions_are_dropped_and_closed_ones_are_gone() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        l.dir("d");
        let store = ListStore::new();
        let s = spaces(&l, &w);
        let first = store.open_dir(&s, &Location::linux("/d"), &ListOptions::default(), 10).unwrap();
        for _ in 0..MAX_SESSIONS {
            store.open_dir(&s, &Location::linux("/d"), &ListOptions::default(), 10).unwrap();
        }
        assert_eq!(store.open_sessions(), MAX_SESSIONS);
        assert_eq!(store.page(&first.id, 0, 10).unwrap_err().kind, FsErrorKind::NotFound);
        let keep = store.open_dir(&s, &Location::linux("/d"), &ListOptions::default(), 10).unwrap();
        store.close(&keep.id);
        assert_eq!(store.page(&keep.id, 0, 10).unwrap_err().kind, FsErrorKind::NotFound);
    }

    #[test]
    fn missing_and_non_directories_are_reported() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        l.file("home/me/file.txt", "x");
        let store = ListStore::new();
        let s = spaces(&l, &w);
        let e = store.open_dir(&s, &Location::linux("/home/nope"), &ListOptions::default(), 10).map(|_| ()).unwrap_err();
        assert_eq!(e.kind, FsErrorKind::NotFound);
        assert!(e.message.contains("/home/nope"));
        let e = store.open_dir(&s, &Location::linux("/home/me/file.txt"), &ListOptions::default(), 10).map(|_| ()).unwrap_err();
        assert!(matches!(e.kind, FsErrorKind::NotADirectory | FsErrorKind::Io), "{e:?}");
        let e = store.open_dir(&s, &Location::linux("relative"), &ListOptions::default(), 10).map(|_| ()).unwrap_err();
        assert_eq!(e.kind, FsErrorKind::InvalidPath);
    }

    #[test]
    fn windows_locations_list_the_shared_folder() {
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        w.file("report.docx", "x");
        w.dir("Sub");
        let store = ListStore::new();
        let head = store.open_dir(&spaces(&l, &w), &Location::windows("documents", &[]), &ListOptions::default(), 10).unwrap();
        assert_eq!(names(&head.page), ["Sub", "report.docx"]);
        assert_eq!(head.crumbs[0].label, "Documents");
        assert_eq!(head.parent, None);
        let sub = store.open_dir(&spaces(&l, &w), &Location::windows("documents", &["Sub"]), &ListOptions::default(), 10).unwrap();
        assert_eq!(sub.parent, Some(Location::windows("documents", &[])));
    }

    #[cfg(unix)]
    #[test]
    fn links_are_listed_as_links_with_their_target() {
        use std::os::unix::fs::symlink;
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        l.dir("home/me/real");
        l.file("home/me/f.txt", "x");
        symlink("real", l.join("home/me/dirlink")).unwrap();
        symlink("f.txt", l.join("home/me/filelink")).unwrap();
        symlink("nowhere", l.join("home/me/broken")).unwrap();
        let store = ListStore::new();
        let head = store.open_dir(&spaces(&l, &w), &Location::linux("/home/me"), &ListOptions::default(), 20).unwrap();
        let get = |n: &str| head.page.entries.iter().find(|e| e.name == n).unwrap();
        assert_eq!(get("dirlink").kind, EntryKind::Symlink);
        assert_eq!(get("dirlink").link_is_dir, Some(true));
        assert_eq!(get("dirlink").link_target.as_deref(), Some("real"));
        assert_eq!(get("filelink").link_is_dir, Some(false));
        assert_eq!(get("broken").link_is_dir, None, "a broken link is shown, not hidden");
        assert!(!get("filelink").open.allowed, "links are not opened directly");
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_directory_gives_a_permission_message() {
        use std::os::unix::fs::PermissionsExt;
        let l = TempDir::new("l");
        let w = TempDir::new("w");
        let d = l.dir("locked");
        l.file("locked/inner", "x");
        fs::set_permissions(&d, fs::Permissions::from_mode(0o000)).unwrap();
        let store = ListStore::new();
        let r = store.open_dir(&spaces(&l, &w), &Location::linux("/locked"), &ListOptions::default(), 10);
        fs::set_permissions(&d, fs::Permissions::from_mode(0o755)).unwrap();
        // (Running as root, e.g. in some containers, bypasses permissions; then there is nothing to assert.)
        if let Err(e) = r {
            assert_eq!(e.kind, FsErrorKind::PermissionDenied);
            assert!(e.message.contains("permission denied"));
        }
    }
}
