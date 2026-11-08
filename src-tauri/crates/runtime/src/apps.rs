//! Linux application discovery from freedesktop Desktop Entries (`*.desktop`).
//!
//! Safe by construction:
//!
//! * Entries are only **read** (through the distribution's file share, which the caller has already
//!   resolved); nothing runs inside the distribution during discovery.
//! * The frontend never sees or sends a command line. It sees [`LinuxApp`] (name, id, icon name)
//!   and later asks to launch an *id*; [`launch_argv`] re-reads that entry and builds an argument
//!   vector (no shell) from its `Exec` key.
//! * No distribution is assumed: the standard XDG application folders are scanned (system, user,
//!   Flatpak, Snap) and whatever exists is used. Nothing is invented — an empty distribution gives
//!   an empty list.
//! * Everything is bounded: entry count, file size, icon size and a time budget.

use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;

pub const MAX_ENTRIES: usize = 1500;
pub const MAX_FILE_BYTES: u64 = 64 * 1024;
pub const MAX_ICON_BYTES: u64 = 256 * 1024;
const TIME_BUDGET: Duration = Duration::from_secs(8);

/// One launchable-looking application, as shown to the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinuxApp {
    /// The desktop-file id (`org.example.App.desktop`). The only thing the frontend sends back.
    pub id: String,
    pub name: String,
    pub generic_name: Option<String>,
    pub comment: Option<String>,
    pub categories: Vec<String>,
    pub keywords: Vec<String>,
    /// The `Icon=` value (a theme name or an absolute path). Only used by the backend to find a file.
    pub icon: Option<String>,
    pub terminal: bool,
    pub launchable: bool,
    /// Why it cannot be launched from here.
    pub reason: Option<String>,
    /// `user`, `system`, `flatpak` or `snap`.
    pub source: String,
}

/// The parts of a Desktop Entry VLINUX uses.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DesktopEntry {
    pub kind: String,
    pub name: String,
    pub generic_name: Option<String>,
    pub comment: Option<String>,
    pub exec: String,
    pub try_exec: Option<String>,
    pub icon: Option<String>,
    pub terminal: bool,
    pub no_display: bool,
    pub hidden: bool,
    pub only_show_in: Vec<String>,
    pub categories: Vec<String>,
    pub keywords: Vec<String>,
}

fn unescape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut it = v.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(';') => out.push(';'),
            Some(o) => {
                out.push('\\');
                out.push(o);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn list(v: &str) -> Vec<String> {
    v.split(';')
        .map(|s| unescape(s.trim()))
        .filter(|s| !s.is_empty())
        .collect()
}

fn truthy(v: &str) -> bool {
    v.trim().eq_ignore_ascii_case("true") || v.trim() == "1"
}

/// Parse the `[Desktop Entry]` group. Localised keys (`Name[de]`) are ignored: the plain key is used.
pub fn parse_desktop(text: &str) -> Option<DesktopEntry> {
    let mut e = DesktopEntry::default();
    let mut in_group = false;
    let mut seen_group = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_group = line == "[Desktop Entry]";
            seen_group |= in_group;
            continue;
        }
        if !in_group {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let (k, v) = (k.trim(), v.trim());
        if k.contains('[') {
            continue;
        }
        match k {
            "Type" => e.kind = v.to_string(),
            "Name" => e.name = unescape(v),
            "GenericName" => e.generic_name = Some(unescape(v)).filter(|s| !s.is_empty()),
            "Comment" => e.comment = Some(unescape(v)).filter(|s| !s.is_empty()),
            "Exec" => e.exec = v.to_string(),
            "TryExec" => e.try_exec = Some(unescape(v)).filter(|s| !s.is_empty()),
            "Icon" => e.icon = Some(unescape(v)).filter(|s| !s.is_empty()),
            "Terminal" => e.terminal = truthy(v),
            "NoDisplay" => e.no_display = truthy(v),
            "Hidden" => e.hidden = truthy(v),
            "OnlyShowIn" => e.only_show_in = list(v),
            "Categories" => e.categories = list(v),
            "Keywords" => e.keywords = list(v),
            _ => {}
        }
    }
    seen_group.then_some(e)
}

/// Split an `Exec` value into an argument vector, per the Desktop Entry specification's quoting
/// rules (double quotes, backslash escapes inside them) and with every field code removed
/// (`%f %F %u %U %d %D %n %N %i %c %k %v %m`; `%%` is a literal percent). No shell is involved, so
/// `$`, backticks, `;` and `&` have no special meaning.
pub fn exec_argv(exec: &str) -> Result<Vec<String>, String> {
    let mut args: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut started = false;
    let mut only_code = true;
    let mut in_quote = false;
    let mut chars = exec.chars().peekable();
    let flush = |cur: &mut String, started: &mut bool, only_code: &mut bool, args: &mut Vec<String>| {
        if *started && !(*only_code && cur.is_empty()) {
            args.push(std::mem::take(cur));
        } else {
            cur.clear();
        }
        *started = false;
        *only_code = true;
    };
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_quote = !in_quote;
                started = true;
                only_code = false;
            }
            '\\' if in_quote => match chars.next() {
                Some(n @ ('"' | '`' | '$' | '\\')) => {
                    cur.push(n);
                    started = true;
                    only_code = false;
                }
                Some(n) => {
                    cur.push('\\');
                    cur.push(n);
                    started = true;
                    only_code = false;
                }
                None => return Err("the command ends with a backslash".into()),
            },
            ' ' | '\t' if !in_quote => flush(&mut cur, &mut started, &mut only_code, &mut args),
            '%' => match chars.next() {
                Some('%') => {
                    cur.push('%');
                    started = true;
                    only_code = false;
                }
                Some('f' | 'F' | 'u' | 'U' | 'd' | 'D' | 'n' | 'N' | 'i' | 'c' | 'k' | 'v' | 'm') => {
                    started = true;
                }
                Some(o) => return Err(format!("unknown field code %{o}")),
                None => return Err("the command ends with a lone %".into()),
            },
            _ => {
                cur.push(c);
                started = true;
                only_code = false;
            }
        }
    }
    if in_quote {
        return Err("an opening quote is never closed".into());
    }
    flush(&mut cur, &mut started, &mut only_code, &mut args);
    if args.is_empty() {
        return Err("the command is empty".into());
    }
    if args.iter().any(|a| a.contains('\0')) {
        return Err("the command contains a NUL character".into());
    }
    Ok(args)
}

/// A desktop-file id: file name only, ending in `.desktop`, no separators.
pub fn valid_app_id(id: &str) -> bool {
    id.len() <= 200
        && id.len() > ".desktop".len()
        && id.ends_with(".desktop")
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+'))
        && !id.starts_with('.')
}

/// Host path of a Linux path under the distribution root; `None` for anything but a clean absolute path.
pub fn host_path(root: &Path, linux: &str) -> Option<PathBuf> {
    if !linux.starts_with('/') || linux.contains('\0') {
        return None;
    }
    let mut p = root.to_path_buf();
    for part in linux.split('/').filter(|s| !s.is_empty()) {
        if part == "." || part == ".." {
            return None;
        }
        let comp = Path::new(part).components().next()?;
        if !matches!(comp, Component::Normal(_)) || part.contains('\\') || part.contains(':') {
            return None;
        }
        p.push(part);
    }
    Some(p)
}

fn entry_exists(root: &Path, linux: &str) -> bool {
    host_path(root, linux).is_some_and(|p| fs::symlink_metadata(p).is_ok())
}

const PROGRAM_DIRS: [&str; 9] = [
    "/usr/local/sbin",
    "/usr/local/bin",
    "/usr/sbin",
    "/usr/bin",
    "/sbin",
    "/bin",
    "/usr/games",
    "/snap/bin",
    "/var/lib/flatpak/exports/bin",
];

fn program_found(root: &Path, home: Option<&str>, cmd: &str) -> bool {
    if cmd.starts_with('/') {
        return entry_exists(root, cmd);
    }
    if cmd.is_empty() || cmd.contains('/') || cmd.contains('\0') {
        return false;
    }
    PROGRAM_DIRS
        .iter()
        .map(|d| (*d).to_string())
        .chain(home.map(|h| format!("{}/.local/bin", h.trim_end_matches('/'))))
        .any(|d| entry_exists(root, &format!("{d}/{cmd}")))
}

fn app_dirs(home: Option<&str>) -> Vec<(String, &'static str)> {
    let mut v: Vec<(String, &'static str)> = Vec::new();
    if let Some(h) = home {
        let h = h.trim_end_matches('/');
        v.push((format!("{h}/.local/share/applications"), "user"));
        v.push((format!("{h}/.local/share/flatpak/exports/share/applications"), "flatpak"));
    }
    v.push(("/var/lib/flatpak/exports/share/applications".into(), "flatpak"));
    v.push(("/var/lib/snapd/desktop/applications".into(), "snap"));
    v.push(("/usr/local/share/applications".into(), "system"));
    v.push(("/usr/share/applications".into(), "system"));
    v
}

/// A discovered entry with its parsed source (the backend keeps this; the frontend gets `app`).
#[derive(Debug, Clone)]
pub struct Found {
    pub app: LinuxApp,
    pub entry: DesktopEntry,
}

#[derive(Debug, Clone, Default)]
pub struct Scan {
    pub found: Vec<Found>,
    /// Folders that exist in this distribution and were read.
    pub scanned: Vec<String>,
    /// Files that were ignored because they were unreadable, too large or malformed.
    pub skipped: usize,
    pub warnings: Vec<String>,
}

fn collect_files(dir: &Path, depth: u32, prefix: &str, out: &mut Vec<(String, PathBuf)>) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for ent in rd.flatten() {
        if out.len() >= MAX_ENTRIES * 2 {
            return;
        }
        let name = ent.file_name().to_string_lossy().into_owned();
        let path = ent.path();
        if name.ends_with(".desktop") {
            out.push((format!("{prefix}{name}"), path));
        } else if depth == 0 && !name.starts_with('.') && !name.contains('.') {
            // A sub-folder `foo/bar.desktop` has the id `foo-bar.desktop`.
            if let Ok(t) = ent.file_type() {
                if t.is_dir() {
                    collect_files(&path, depth + 1, &format!("{prefix}{name}-"), out);
                }
            }
        }
    }
}

/// Read every application the distribution registers. `root` is the host path of its file share,
/// `home` the Linux home directory when known.
pub fn scan(root: &Path, home: Option<&str>) -> Scan {
    let started = Instant::now();
    let mut s = Scan::default();
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut seen_pairs: HashSet<(String, String)> = HashSet::new();
    'dirs: for (linux_dir, source) in app_dirs(home) {
        let Some(host) = host_path(root, &linux_dir) else {
            continue;
        };
        if !host.is_dir() {
            continue;
        }
        s.scanned.push(linux_dir.clone());
        let mut files = Vec::new();
        collect_files(&host, 0, "", &mut files);
        files.sort();
        for (id, path) in files {
            if started.elapsed() > TIME_BUDGET {
                s.warnings.push("Reading the application folders took too long; the list may be incomplete.".into());
                break 'dirs;
            }
            if s.found.len() >= MAX_ENTRIES {
                s.warnings.push(format!("Only the first {MAX_ENTRIES} applications are shown."));
                break 'dirs;
            }
            if !valid_app_id(&id) {
                s.skipped += 1;
                continue;
            }
            // An entry in an earlier (higher priority) folder hides the same id further down,
            // even when that earlier entry is itself hidden.
            if !seen_ids.insert(id.clone()) {
                continue;
            }
            let Ok(meta) = fs::metadata(&path) else {
                s.skipped += 1;
                continue;
            };
            if meta.len() > MAX_FILE_BYTES || !meta.is_file() {
                s.skipped += 1;
                continue;
            }
            let Ok(bytes) = fs::read(&path) else {
                s.skipped += 1;
                continue;
            };
            let text = String::from_utf8_lossy(&bytes);
            let Some(entry) = parse_desktop(&text) else {
                s.skipped += 1;
                continue;
            };
            if entry.kind != "Application"
                || entry.no_display
                || entry.hidden
                || entry.name.trim().is_empty()
                || !entry.only_show_in.is_empty()
            {
                continue;
            }
            let (launchable, reason) = judge(root, home, &entry);
            if !seen_pairs.insert((entry.name.to_lowercase(), entry.exec.clone())) {
                continue; // the same program listed under two ids
            }
            s.found.push(Found {
                app: LinuxApp {
                    id,
                    name: entry.name.trim().to_string(),
                    generic_name: entry.generic_name.clone(),
                    comment: entry.comment.clone(),
                    categories: entry.categories.clone(),
                    keywords: entry.keywords.clone(),
                    icon: entry.icon.clone(),
                    terminal: entry.terminal,
                    launchable,
                    reason,
                    source: source.to_string(),
                },
                entry,
            });
        }
    }
    s.found
        .sort_by_key(|f| (f.app.name.to_lowercase(), f.app.id.clone()));
    s
}

fn judge(root: &Path, home: Option<&str>, e: &DesktopEntry) -> (bool, Option<String>) {
    if e.terminal {
        return (
            false,
            Some("This program runs inside a terminal. Open it from the Terminal app.".into()),
        );
    }
    let argv = match exec_argv(&e.exec) {
        Ok(a) => a,
        Err(m) => return (false, Some(format!("Its launch command is not valid ({m})."))),
    };
    if let Some(t) = &e.try_exec {
        if !program_found(root, home, t) {
            return (false, Some(format!("'{t}' is not installed in this distribution.")));
        }
    }
    if let Some(first) = argv.first() {
        if first.starts_with('/') && !entry_exists(root, first) {
            return (false, Some(format!("'{first}' does not exist in this distribution.")));
        }
    }
    (true, None)
}

/// Re-read the entry with this id and build the argument vector to run it (never through a shell).
pub fn launch_argv(root: &Path, home: Option<&str>, id: &str) -> Result<(LinuxApp, Vec<String>), String> {
    if !valid_app_id(id) {
        return Err("That is not a valid application id.".into());
    }
    let sc = scan(root, home);
    let Some(f) = sc.found.into_iter().find(|f| f.app.id == id) else {
        return Err("That application is no longer installed in this distribution.".into());
    };
    if !f.app.launchable {
        return Err(f
            .app
            .reason
            .clone()
            .unwrap_or_else(|| "This application cannot be launched from VLINUX.".into()));
    }
    let argv = exec_argv(&f.entry.exec)?;
    Ok((f.app, argv))
}

fn icon_name_ok(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 120
        && n.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+'))
        && !n.starts_with('.')
}

const ICON_ROOTS: [&str; 6] = [
    "/usr/share/",
    "/usr/local/share/",
    "/opt/",
    "/var/lib/flatpak/",
    "/var/lib/snapd/",
    "/snap/",
];

fn path_icon_allowed(p: &str, home: Option<&str>) -> bool {
    ICON_ROOTS.iter().any(|r| p.starts_with(r))
        || home.is_some_and(|h| p.starts_with(&format!("{}/.local/share/", h.trim_end_matches('/'))))
}

/// `image/png` or `image/svg+xml` when the bytes look like one, else `None`.
pub fn sniff_image(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        return Some("image/png");
    }
    let head = &b[..b.len().min(2048)];
    let text = String::from_utf8_lossy(head);
    if text.contains("<svg") {
        return Some("image/svg+xml");
    }
    None
}

/// Find an icon file for an `Icon=` value: an absolute path inside the usual application folders,
/// or a theme name looked up in pixmaps and the hicolor theme. Returns the image type and bytes.
pub fn find_icon(root: &Path, home: Option<&str>, icon: &str) -> Option<(&'static str, Vec<u8>)> {
    let mut candidates: Vec<String> = Vec::new();
    if icon.starts_with('/') {
        if !path_icon_allowed(icon, home) {
            return None;
        }
        candidates.push(icon.to_string());
    } else if icon_name_ok(icon) {
        let mut bases: Vec<String> = vec!["/usr/share".into(), "/usr/local/share".into()];
        if let Some(h) = home {
            bases.push(format!("{}/.local/share", h.trim_end_matches('/')));
        }
        bases.push("/var/lib/flatpak/exports/share".into());
        for b in &bases {
            for size in ["scalable", "256x256", "128x128", "96x96", "64x64", "48x48", "512x512", "32x32"] {
                for ext in ["svg", "png"] {
                    candidates.push(format!("{b}/icons/hicolor/{size}/apps/{icon}.{ext}"));
                }
            }
            for ext in ["svg", "png"] {
                candidates.push(format!("{b}/pixmaps/{icon}.{ext}"));
            }
        }
    } else {
        return None;
    }
    for c in candidates {
        let Some(p) = host_path(root, &c) else {
            continue;
        };
        let Ok(m) = fs::metadata(&p) else {
            continue;
        };
        if !m.is_file() || m.len() == 0 || m.len() > MAX_ICON_BYTES {
            continue;
        }
        let Ok(bytes) = fs::read(&p) else {
            continue;
        };
        if let Some(mime) = sniff_image(&bytes) {
            return Some((mime, bytes));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::testutil::TempDir;

    fn desk(name: &str, exec: &str, extra: &str) -> String {
        format!("[Desktop Entry]\nType=Application\nName={name}\nExec={exec}\n{extra}")
    }

    fn put(root: &TempDir, rel: &str, text: &str) {
        root.file(rel, text);
    }

    #[test]
    fn parses_the_desktop_entry_group_and_ignores_actions_and_translations() {
        let e = parse_desktop(
            "# comment\n[Desktop Entry]\nType=Application\nName=Files\nName[de]=Dateien\nExec=files %U\nIcon=files\nCategories=System;FileManager;\nKeywords=folder;browse;\nTerminal=false\n\n[Desktop Action new]\nName=New window\nExec=files --new\n",
        )
        .unwrap();
        assert_eq!(e.name, "Files");
        assert_eq!(e.exec, "files %U");
        assert_eq!(e.categories, ["System", "FileManager"]);
        assert_eq!(e.keywords, ["folder", "browse"]);
        assert!(!e.terminal);
        assert!(parse_desktop("Name=x\nExec=y\n").is_none(), "no [Desktop Entry] group");
    }

    #[test]
    fn exec_quoting_and_field_codes() {
        assert_eq!(exec_argv("firefox %u").unwrap(), ["firefox"]);
        assert_eq!(exec_argv("env FOO=1 app --flag %F").unwrap(), ["env", "FOO=1", "app", "--flag"]);
        assert_eq!(
            exec_argv(r#""/opt/My App/run" --name "a b" %U"#).unwrap(),
            ["/opt/My App/run", "--name", "a b"]
        );
        assert_eq!(exec_argv("app --file=%f").unwrap(), ["app", "--file="]);
        assert_eq!(exec_argv("app 100%% done").unwrap(), ["app", "100%", "done"]);
        assert_eq!(exec_argv(r#"app "say \"hi\" \$HOME""#).unwrap(), ["app", r#"say "hi" $HOME"#]);
        // No shell: metacharacters are just text.
        assert_eq!(exec_argv("app a;b $(x) `y` &").unwrap(), ["app", "a;b", "$(x)", "`y`", "&"]);
        assert!(exec_argv("").is_err());
        assert!(exec_argv("%U").is_err());
        assert!(exec_argv("app \"open").is_err());
        assert!(exec_argv("app %z").is_err());
    }

    #[test]
    fn ids_and_paths_are_validated() {
        assert!(valid_app_id("org.gnome.Nautilus.desktop"));
        assert!(valid_app_id("foo-bar_1+2.desktop"));
        for bad in ["", ".desktop", "a/b.desktop", "a b.desktop", "x.txt", "../x.desktop", ".hidden.desktop", "a;b.desktop"] {
            assert!(!valid_app_id(bad), "{bad}");
        }
        let r = Path::new("/r");
        assert_eq!(host_path(r, "/usr/bin/x"), Some(PathBuf::from("/r/usr/bin/x")));
        assert_eq!(host_path(r, "/usr/../etc"), None);
        assert_eq!(host_path(r, "usr/bin"), None);
        assert_eq!(host_path(r, "/a\\b"), None);
        assert_eq!(host_path(r, "/c:/x"), None);
    }

    #[test]
    fn discovers_applications_across_the_standard_folders_without_assuming_a_distribution() {
        let root = TempDir::new("apps");
        put(&root, "usr/share/applications/editor.desktop", &desk("Text Editor", "/usr/bin/editor %F", "Icon=editor\nCategories=Utility;\n"));
        root.file("usr/bin/editor", "#!/bin/sh");
        put(&root, "usr/share/applications/hidden.desktop", &desk("Hidden", "/usr/bin/editor", "NoDisplay=true\n"));
        put(&root, "usr/share/applications/gnome-only.desktop", &desk("Gnome Only", "/usr/bin/editor", "OnlyShowIn=GNOME;\n"));
        put(&root, "usr/share/applications/term.desktop", &desk("Htop", "htop", "Terminal=true\n"));
        put(&root, "usr/share/applications/missing.desktop", &desk("Ghost", "/usr/bin/ghost", ""));
        put(&root, "usr/share/applications/tryexec.desktop", &desk("TryIt", "/usr/bin/editor", "TryExec=nothere\n"));
        put(&root, "var/lib/flatpak/exports/share/applications/org.x.Fly.desktop", &desk("Flat App", "/usr/bin/editor --fly", ""));
        put(&root, "home/me/.local/share/applications/editor.desktop", &desk("My Editor", "/usr/bin/editor --mine", ""));
        put(&root, "usr/share/applications/dup-a.desktop", &desk("Same Thing", "/usr/bin/editor --same", ""));
        put(&root, "usr/share/applications/dup-b.desktop", &desk("same thing", "/usr/bin/editor --same", ""));
        put(&root, "usr/share/applications/bad.desktop", "not a desktop file");
        put(&root, "usr/share/applications/kind.desktop", "[Desktop Entry]\nType=Link\nName=Web\nURL=x\n");
        let s = scan(root.path(), Some("/home/me"));
        let names: Vec<&str> = s.found.iter().map(|f| f.app.name.as_str()).collect();
        // The user's editor.desktop masks the system one with the same id.
        assert!(names.contains(&"My Editor") && !names.contains(&"Text Editor"), "{names:?}");
        assert!(names.contains(&"Flat App"));
        for gone in ["Hidden", "Gnome Only", "Web"] {
            assert!(!names.contains(&gone), "{gone}");
        }
        let get = |n: &str| s.found.iter().find(|f| f.app.name == n).unwrap();
        assert!(!get("Htop").app.launchable && get("Htop").app.reason.as_deref().unwrap().contains("Terminal"));
        assert!(!get("Ghost").app.launchable && get("Ghost").app.reason.as_deref().unwrap().contains("does not exist"));
        assert!(!get("TryIt").app.launchable);
        assert!(get("Flat App").app.launchable);
        assert_eq!(get("Flat App").app.source, "flatpak");
        assert_eq!(get("My Editor").app.source, "user");
        // The same program under two ids appears once.
        assert_eq!(names.iter().filter(|n| n.eq_ignore_ascii_case("same thing")).count(), 1);
        assert_eq!(s.skipped, 1, "only the malformed file counts as skipped");
        assert!(s.scanned.iter().any(|d| d == "/usr/share/applications"));
        // Sorted by name, case-insensitively.
        let mut sorted = names.clone();
        sorted.sort_by_key(|n| n.to_lowercase());
        assert_eq!(names, sorted);
    }

    #[test]
    fn an_empty_distribution_gives_an_empty_list_not_invented_entries() {
        let root = TempDir::new("noapps");
        let s = scan(root.path(), Some("/home/me"));
        assert!(s.found.is_empty() && s.scanned.is_empty());
    }

    #[test]
    fn a_hidden_entry_in_a_higher_priority_folder_masks_the_system_one() {
        let root = TempDir::new("mask");
        root.file("usr/bin/a", "x");
        put(&root, "usr/share/applications/a.desktop", &desk("Visible", "/usr/bin/a", ""));
        put(&root, "home/me/.local/share/applications/a.desktop", &desk("Visible", "/usr/bin/a", "Hidden=true\n"));
        assert!(scan(root.path(), Some("/home/me")).found.is_empty());
    }

    #[test]
    fn oversized_files_are_skipped_and_counted() {
        let root = TempDir::new("big");
        let big = format!("[Desktop Entry]\nType=Application\nName=Big\nExec=/usr/bin/x\n#{}\n", "x".repeat(70_000));
        put(&root, "usr/share/applications/big.desktop", &big);
        let s = scan(root.path(), None);
        assert!(s.found.is_empty());
        assert_eq!(s.skipped, 1);
    }

    #[test]
    fn launching_rebuilds_the_command_from_the_entry_and_refuses_what_is_not_launchable() {
        let root = TempDir::new("launch");
        root.file("usr/bin/editor", "x");
        put(&root, "usr/share/applications/editor.desktop", &desk("Editor", "/usr/bin/editor --new-window %F", ""));
        put(&root, "usr/share/applications/term.desktop", &desk("Htop", "htop", "Terminal=true\n"));
        let (app, argv) = launch_argv(root.path(), None, "editor.desktop").unwrap();
        assert_eq!(app.name, "Editor");
        assert_eq!(argv, ["/usr/bin/editor", "--new-window"]);
        assert!(launch_argv(root.path(), None, "term.desktop").unwrap_err().contains("Terminal"));
        assert!(launch_argv(root.path(), None, "nothere.desktop").unwrap_err().contains("no longer installed"));
        assert!(launch_argv(root.path(), None, "../etc/passwd").is_err());
        assert!(launch_argv(root.path(), None, "a b.desktop").is_err());
    }

    fn png() -> Vec<u8> {
        let mut v = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        v.extend_from_slice(&[0; 32]);
        v
    }

    #[test]
    fn icons_are_found_by_name_or_path_and_only_as_images_inside_known_folders() {
        let root = TempDir::new("icons");
        root.bytes("usr/share/icons/hicolor/48x48/apps/editor.png", &png());
        root.file("usr/share/pixmaps/vec.svg", "<?xml version=\"1.0\"?><svg xmlns=\"http://www.w3.org/2000/svg\"/>");
        root.file("usr/share/pixmaps/fake.png", "this is not an image");
        root.file("etc/secret.png", "x");
        root.bytes("opt/app/icon.png", &png());
        let (mime, b) = find_icon(root.path(), None, "editor").unwrap();
        assert_eq!(mime, "image/png");
        assert!(b.len() > 8);
        assert_eq!(find_icon(root.path(), None, "vec").unwrap().0, "image/svg+xml");
        assert!(find_icon(root.path(), None, "fake").is_none(), "content must be an image");
        assert!(find_icon(root.path(), None, "/opt/app/icon.png").is_some());
        assert!(find_icon(root.path(), None, "/etc/secret.png").is_none(), "outside the icon folders");
        assert!(find_icon(root.path(), None, "../../etc/secret").is_none());
        assert!(find_icon(root.path(), None, "/usr/share/../../etc/secret.png").is_none());
        assert!(find_icon(root.path(), None, "nothing").is_none());
    }

    #[test]
    fn oversized_icons_are_ignored() {
        let root = TempDir::new("bigicon");
        let mut v = png();
        v.extend(std::iter::repeat(0u8).take(300_000));
        root.bytes("usr/share/pixmaps/huge.png", &v);
        assert!(find_icon(root.path(), None, "huge").is_none());
    }
}
