//! Reviewable, reversible editing of a distribution's `/etc/wsl.conf` (M5).
//!
//! The file belongs to the distribution, not to VLINUX, so the rules are strict:
//!
//! * **Read first.** The current file is parsed by a line-based, comment-preserving editor
//!   ([`IniDoc`]). Unknown keys, unknown sections, comments, blank lines, key spelling and line
//!   endings survive an edit untouched; only the lines for the requested settings change.
//! * **Preview, then confirm.** [`plan`] returns the exact new text and a line diff. Nothing is
//!   written until the caller applies it, and the apply is refused if the file changed after the
//!   preview (fingerprint check).
//! * **Backup first, restore later.** [`apply`] stores the previous content in the VLINUX app data
//!   folder before writing, writes through a temp file + rename, reads the result back, and puts the
//!   original back if the read-back differs. [`restore`] is itself backed up, so it is reversible.
//! * **Never elevate.** A distribution's `/etc` is usually root-owned; when the write is denied the
//!   result is `NotWritable` with the proposed text and manual instructions. VLINUX does not use
//!   `sudo`, `wsl -u root` or anything similar.
//! * **No claim of effect.** A saved file only takes effect after that distribution restarts, and
//!   VLINUX does not restart or stop anything. The outcome says so, and says it was *not* verified
//!   active.
//! * Only the distribution's own file is handled here. The Windows `.wslconfig` is read-only (see
//!   `winconf`).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::{DistroKind, DistroState, RuntimeStatus, WslReport};
use crate::parse::valid_distro_name;
use crate::shared::distro::valid_exec_path;

pub const CONF_PATH: &str = "/etc/wsl.conf";
pub const MAX_CONF_BYTES: usize = 64 * 1024;
const MAX_LINES: usize = 2000;
const KEEP_PER_DISTRO: usize = 20;
const BACKUPS_FILE: &str = "wslconf-backups.json";

// ---------------------------------------------------------------------------------------------
// Comment-preserving INI document
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    Blank,
    Comment,
    Section(String),
    /// `prefix` = everything up to and including `=` and the spaces after it.
    Pair { key: String, value: String, prefix: String, tail: String },
    /// Not understood; makes the document "malformed".
    Other,
}

#[derive(Debug, Clone)]
struct Line {
    raw: String,
    kind: Kind,
}

#[derive(Debug, Clone)]
pub struct IniDoc {
    lines: Vec<Line>,
    eol: &'static str,
    trailing_newline: bool,
    /// 1-based line numbers VLINUX could not understand.
    pub malformed: Vec<usize>,
}

fn parse_line(raw: &str, in_section: bool) -> Kind {
    let t = raw.trim();
    if t.is_empty() {
        return Kind::Blank;
    }
    if t.starts_with('#') || t.starts_with(';') {
        return Kind::Comment;
    }
    if let Some(rest) = t.strip_prefix('[') {
        return match rest.strip_suffix(']') {
            Some(name) if !name.trim().is_empty() && !name.contains(['[', ']']) => Kind::Section(name.trim().to_string()),
            _ => Kind::Other,
        };
    }
    if !in_section {
        return Kind::Other;
    }
    let Some(eq) = raw.find('=') else { return Kind::Other };
    let key = raw[..eq].trim();
    if key.is_empty() || key.contains(char::is_whitespace) {
        return Kind::Other;
    }
    let after = &raw[eq + 1..];
    let lead = after.len() - after.trim_start().len();
    let prefix = format!("{}{}", &raw[..eq + 1], &after[..lead]);
    let rest = &after[lead..];
    // An unquoted value ends at a ` #` inline comment; quoted values are kept whole.
    let (value, tail) = if rest.starts_with('"') {
        match rest[1..].find('"') {
            Some(end) => (rest[..end + 2].to_string(), rest[end + 2..].to_string()),
            None => (rest.trim_end().to_string(), String::new()),
        }
    } else {
        match rest.find(" #").or_else(|| rest.find("\t#")) {
            Some(i) => (rest[..i].trim_end().to_string(), rest[rest[..i].trim_end().len()..].to_string()),
            None => (rest.trim_end().to_string(), rest[rest.trim_end().len()..].to_string()),
        }
    };
    Kind::Pair { key: key.to_string(), value, prefix, tail }
}

fn unquote(v: &str) -> String {
    let t = v.trim();
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        t[1..t.len() - 1].to_string()
    } else {
        t.to_string()
    }
}

impl IniDoc {
    pub fn parse(text: &str) -> Self {
        let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
        let trailing_newline = text.ends_with('\n');
        let body = text.strip_suffix('\n').unwrap_or(text);
        let mut lines = Vec::new();
        let mut malformed = Vec::new();
        let mut in_section = false;
        if !text.is_empty() {
            for (i, raw) in body.split('\n').enumerate() {
                let raw = raw.strip_suffix('\r').unwrap_or(raw);
                let kind = parse_line(raw, in_section);
                match &kind {
                    Kind::Section(_) => in_section = true,
                    Kind::Other => malformed.push(i + 1),
                    _ => {}
                }
                lines.push(Line { raw: raw.to_string(), kind });
            }
        }
        Self { lines, eol, trailing_newline: trailing_newline || text.is_empty(), malformed }
    }

    pub fn render(&self) -> String {
        if self.lines.is_empty() {
            return String::new();
        }
        let mut s = self.lines.iter().map(|l| l.raw.as_str()).collect::<Vec<_>>().join(self.eol);
        if self.trailing_newline {
            s.push_str(self.eol);
        }
        s
    }

    pub fn is_malformed(&self) -> bool {
        !self.malformed.is_empty()
    }

    /// Case-insensitive lookup; the last occurrence wins.
    pub fn get(&self, section: &str, key: &str) -> Option<String> {
        let mut cur = String::new();
        let mut found = None;
        for l in &self.lines {
            match &l.kind {
                Kind::Section(n) => cur = n.clone(),
                Kind::Pair { key: k, value, .. } if cur.eq_ignore_ascii_case(section) && k.eq_ignore_ascii_case(key) => found = Some(unquote(value)),
                _ => {}
            }
        }
        found
    }

    /// Every `section.key` present, in file order.
    pub fn keys(&self) -> Vec<(String, String)> {
        let mut cur = String::new();
        let mut out = Vec::new();
        for l in &self.lines {
            match &l.kind {
                Kind::Section(n) => cur = n.clone(),
                Kind::Pair { key, .. } => out.push((cur.clone(), key.clone())),
                _ => {}
            }
        }
        out
    }

    pub fn set(&mut self, section: &str, key: &str, value: &str) {
        let mut cur = String::new();
        let mut target: Option<usize> = None;
        let mut section_end: Option<usize> = None;
        for (i, l) in self.lines.iter().enumerate() {
            match &l.kind {
                Kind::Section(n) => {
                    cur = n.clone();
                    if cur.eq_ignore_ascii_case(section) {
                        section_end = Some(i);
                    }
                }
                Kind::Pair { key: k, .. } if cur.eq_ignore_ascii_case(section) => {
                    section_end = Some(i);
                    if k.eq_ignore_ascii_case(key) {
                        target = Some(i);
                    }
                }
                Kind::Comment | Kind::Other if cur.eq_ignore_ascii_case(section) => {}
                _ => {}
            }
        }
        if let Some(i) = target {
            if let Kind::Pair { key: k, prefix, tail, .. } = self.lines[i].kind.clone() {
                let raw = format!("{prefix}{value}{tail}");
                self.lines[i] = Line { raw, kind: Kind::Pair { key: k, value: value.to_string(), prefix, tail } };
            }
            return;
        }
        let new_line = Line { raw: format!("{key}={value}"), kind: Kind::Pair { key: key.to_string(), value: value.to_string(), prefix: format!("{key}="), tail: String::new() } };
        match section_end {
            Some(i) => self.lines.insert(i + 1, new_line),
            None => {
                if self.lines.last().is_some_and(|l| l.kind != Kind::Blank) {
                    self.lines.push(Line { raw: String::new(), kind: Kind::Blank });
                }
                self.lines.push(Line { raw: format!("[{section}]"), kind: Kind::Section(section.to_string()) });
                self.lines.push(new_line);
            }
        }
        self.trailing_newline = true;
    }

    /// Remove the setting (all occurrences). Returns whether anything was removed. An emptied
    /// section header is left alone: removing lines the user did not ask about is not our call.
    pub fn unset(&mut self, section: &str, key: &str) -> bool {
        let mut cur = String::new();
        let before = self.lines.len();
        let mut keep = Vec::with_capacity(before);
        for l in std::mem::take(&mut self.lines) {
            if let Kind::Section(n) = &l.kind {
                cur = n.clone();
            }
            let drop = matches!(&l.kind, Kind::Pair { key: k, .. } if cur.eq_ignore_ascii_case(section) && k.eq_ignore_ascii_case(key));
            if !drop {
                keep.push(l);
            }
        }
        self.lines = keep;
        self.lines.len() != before
    }
}

// ---------------------------------------------------------------------------------------------
// Curated settings
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueKind {
    Bool,
    Path,
    Username,
}

#[derive(Debug, Clone, Copy)]
pub struct SettingDef {
    pub id: &'static str,
    pub section: &'static str,
    pub key: &'static str,
    pub kind: ValueKind,
    pub label: &'static str,
    pub help: &'static str,
    pub default_note: &'static str,
}

pub static SETTINGS: [SettingDef; 6] = [
    SettingDef { id: "automount.enabled", section: "automount", key: "enabled", kind: ValueKind::Bool, label: "Mount Windows drives", help: "Make Windows drives (C:, D:, …) available inside the distribution.", default_note: "WSL default: on" },
    SettingDef { id: "automount.root", section: "automount", key: "root", kind: ValueKind::Path, label: "Drive mount folder", help: "Where Windows drives appear. VLINUX shared-folder links point into this folder.", default_note: "WSL default: /mnt/" },
    SettingDef { id: "interop.enabled", section: "interop", key: "enabled", kind: ValueKind::Bool, label: "Run Windows programs", help: "Let Linux start Windows programs (including opening files with Windows apps).", default_note: "WSL default: on" },
    SettingDef { id: "interop.appendWindowsPath", section: "interop", key: "appendWindowsPath", kind: ValueKind::Bool, label: "Windows PATH in Linux", help: "Add the Windows PATH to the distribution's PATH.", default_note: "WSL default: on" },
    SettingDef { id: "boot.systemd", section: "boot", key: "systemd", kind: ValueKind::Bool, label: "systemd at startup", help: "Run systemd as the init process (WSL 2 only).", default_note: "WSL default: off unless the distribution sets it" },
    SettingDef { id: "user.default", section: "user", key: "default", kind: ValueKind::Username, label: "Default user", help: "The user a new terminal opens as. It must already exist in the distribution.", default_note: "Not set: the user chosen when the distribution was set up" },
];

pub fn setting(id: &str) -> Option<&'static SettingDef> {
    SETTINGS.iter().find(|s| s.id == id)
}

fn valid_username(v: &str) -> bool {
    let mut c = v.chars();
    matches!(c.next(), Some(f) if f.is_ascii_lowercase() || f == '_') && v.len() <= 32 && c.all(|x| x.is_ascii_lowercase() || x.is_ascii_digit() || x == '_' || x == '-')
}

fn validate_value(def: &SettingDef, v: &str) -> Result<(), ConfError> {
    let ok = match def.kind {
        ValueKind::Bool => v == "true" || v == "false",
        ValueKind::Path => valid_exec_path(v) && v != "/" && !v.contains(['#', ' ']),
        ValueKind::Username => valid_username(v),
    };
    if ok {
        Ok(())
    } else {
        Err(ConfError::Invalid(match def.kind {
            ValueKind::Bool => format!("'{}' must be true or false.", def.label),
            ValueKind::Path => format!("'{}' must be an absolute Linux folder such as /mnt/ (no spaces, quotes, '..' or '#').", def.label),
            ValueKind::Username => format!("'{}' must be a valid Linux user name (lower-case letters, digits, '_' and '-').", def.label),
        }))
    }
}

/// FNV-1a 64-bit fingerprint of the file content (change detection, not security).
pub fn fingerprint(text: Option<&str>) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    match text {
        // A missing file and an empty file must not look the same.
        None => h ^= 0xff,
        Some(t) => {
            for b in t.bytes() {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x100000001b3);
            }
        }
    }
    format!("{h:016x}")
}

// ---------------------------------------------------------------------------------------------
// Errors and views
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfError {
    /// Reading or writing the file needs the distribution to be running (reaching its files starts it).
    NotRunning(String),
    Invalid(String),
    /// The file has lines VLINUX cannot parse; it is not edited.
    Malformed(Vec<usize>),
    TooLarge,
    /// The file changed after the preview.
    Changed,
    NoChange,
    Io(String),
    BackupFailed(String),
    NotFound(String),
    /// Restore refused: the file differs from what VLINUX wrote.
    Diverged,
    /// The write failed and the previous content was put back (or was never replaced).
    VerifyFailed { restored: bool, detail: String },
}

impl std::fmt::Display for ConfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfError::NotRunning(n) => write!(f, "'{n}' is not running. Reaching its files would start it, so start it first if you want to read or change its configuration."),
            ConfError::Invalid(m) | ConfError::Io(m) | ConfError::NotFound(m) => f.write_str(m),
            ConfError::Malformed(l) => write!(f, "/etc/wsl.conf has lines VLINUX cannot understand (line {}). It was not changed — fix it by hand first.", l.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(", ")),
            ConfError::TooLarge => f.write_str("/etc/wsl.conf is too large to edit safely here."),
            ConfError::Changed => f.write_str("The file changed after the preview. Nothing was written — preview the change again."),
            ConfError::NoChange => f.write_str("That would not change the file."),
            ConfError::BackupFailed(m) => write!(f, "Could not save a backup, so nothing was written: {m}"),
            ConfError::Diverged => f.write_str("The file has changed since VLINUX wrote it, so restoring would discard someone else's edit. Nothing was changed. You can force the restore if you are sure."),
            ConfError::VerifyFailed { restored, detail } => write!(f, "The file did not read back as written ({detail}). {}", if *restored { "The previous content was put back." } else { "The previous content could NOT be put back automatically — restore it from the backup list." }),
        }
    }
}

impl std::error::Error for ConfError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SettingValue {
    pub id: String,
    pub label: String,
    pub help: String,
    pub kind: ValueKind,
    pub section: String,
    pub key: String,
    /// What the file says; `None` = not set (WSL's own default applies).
    pub value: Option<String>,
    pub default_note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfView {
    pub distro: String,
    pub path: String,
    pub exists: bool,
    pub malformed: Vec<usize>,
    pub settings: Vec<SettingValue>,
    /// Keys VLINUX does not manage; kept as they are on every edit.
    pub other_keys: Vec<String>,
    pub fingerprint: String,
    pub size: usize,
    pub editable: bool,
    pub notes: Vec<String>,
}

/// Read-only view of the current file contents.
pub fn view(distro: &str, text: Option<&str>) -> ConfView {
    let doc = IniDoc::parse(text.unwrap_or(""));
    let managed: Vec<(String, String)> = SETTINGS.iter().map(|s| (s.section.to_ascii_lowercase(), s.key.to_ascii_lowercase())).collect();
    let other_keys: Vec<String> = doc
        .keys()
        .into_iter()
        .filter(|(s, k)| !managed.contains(&(s.to_ascii_lowercase(), k.to_ascii_lowercase())))
        .map(|(s, k)| format!("{s}.{k}"))
        .take(100)
        .collect();
    let size = text.map(|t| t.len()).unwrap_or(0);
    let mut notes = Vec::new();
    if text.is_none() {
        notes.push("The distribution has no /etc/wsl.conf. WSL's defaults apply; saving a change creates the file.".into());
    }
    let too_large = size > MAX_CONF_BYTES || text.is_some_and(|t| t.lines().count() > MAX_LINES);
    if too_large {
        notes.push("The file is too large to edit safely here.".into());
    }
    ConfView {
        distro: distro.to_string(),
        path: CONF_PATH.into(),
        exists: text.is_some(),
        malformed: doc.malformed.clone(),
        settings: SETTINGS
            .iter()
            .map(|s| SettingValue {
                id: s.id.into(),
                label: s.label.into(),
                help: s.help.into(),
                kind: s.kind,
                section: s.section.into(),
                key: s.key.into(),
                value: doc.get(s.section, s.key),
                default_note: s.default_note.into(),
            })
            .collect(),
        other_keys,
        fingerprint: fingerprint(text),
        size,
        editable: !doc.is_malformed() && !too_large,
        notes,
    }
}

// ---------------------------------------------------------------------------------------------
// Plan (preview)
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Change {
    pub id: String,
    /// `None` = remove the setting (go back to WSL's default).
    pub value: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffKind {
    Same,
    Add,
    Remove,
    /// Lines left out between two changes.
    Gap,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiffLine {
    pub kind: DiffKind,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Plan {
    pub distro: String,
    /// Fingerprint of the file this plan was made from; the apply must present it.
    pub based_on: String,
    pub creates_file: bool,
    pub new_text: String,
    pub diff: Vec<DiffLine>,
    pub summary: Vec<String>,
    pub warnings: Vec<String>,
    pub restart_note: String,
}

pub const RESTART_NOTE: &str = "A saved change takes effect only after this distribution restarts. VLINUX will not restart or stop it for you, and it cannot confirm the change has taken effect until you have restarted it.";

/// Build the new file text for `changes` without writing anything.
pub fn plan(distro: &str, current: Option<&str>, changes: &[Change], wsl_version: u8) -> Result<Plan, ConfError> {
    if changes.is_empty() {
        return Err(ConfError::NoChange);
    }
    if let Some(t) = current {
        if t.len() > MAX_CONF_BYTES || t.lines().count() > MAX_LINES {
            return Err(ConfError::TooLarge);
        }
    }
    let mut doc = IniDoc::parse(current.unwrap_or(""));
    if doc.is_malformed() {
        return Err(ConfError::Malformed(doc.malformed.clone()));
    }
    let mut seen: Vec<&str> = Vec::new();
    let mut summary = Vec::new();
    let mut warnings = Vec::new();
    for c in changes {
        let def = setting(&c.id).ok_or_else(|| ConfError::Invalid(format!("'{}' is not a setting VLINUX manages.", c.id.chars().take(40).collect::<String>())))?;
        if seen.contains(&def.id) {
            return Err(ConfError::Invalid(format!("'{}' was given twice.", def.label)));
        }
        seen.push(def.id);
        let before = doc.get(def.section, def.key);
        match &c.value {
            Some(v) => {
                validate_value(def, v)?;
                if before.as_deref() == Some(v.as_str()) {
                    continue;
                }
                doc.set(def.section, def.key, v);
                summary.push(match &before {
                    Some(b) => format!("{}: {b} → {v}", def.label),
                    None => format!("{}: not set → {v}", def.label),
                });
                warnings.extend(warnings_for(def.id, v, wsl_version));
            }
            None => {
                if before.is_none() {
                    continue;
                }
                doc.unset(def.section, def.key);
                summary.push(format!("{}: {} → removed (WSL default)", def.label, before.unwrap_or_default()));
            }
        }
    }
    let new_text = doc.render();
    if Some(new_text.as_str()) == current || (current.is_none() && new_text.is_empty()) {
        return Err(ConfError::NoChange);
    }
    if new_text.len() > MAX_CONF_BYTES {
        return Err(ConfError::TooLarge);
    }
    // The result must still be a file VLINUX can read back.
    let check = IniDoc::parse(&new_text);
    if check.is_malformed() {
        return Err(ConfError::Malformed(check.malformed));
    }
    Ok(Plan {
        distro: distro.to_string(),
        based_on: fingerprint(current),
        creates_file: current.is_none(),
        diff: diff_lines(current.unwrap_or(""), &new_text),
        new_text,
        summary,
        warnings,
        restart_note: RESTART_NOTE.into(),
    })
}

fn warnings_for(id: &str, value: &str, wsl_version: u8) -> Vec<String> {
    let mut w = Vec::new();
    match (id, value) {
        ("automount.root", _) => w.push("VLINUX shared-folder links point into the drive mount folder. After changing it, existing links stop working until you re-apply them from Settings → Shared folders.".to_string()),
        ("automount.enabled", "false") => w.push("Windows drives will no longer be mounted, so VLINUX shared-folder links will not work in this distribution.".to_string()),
        ("interop.enabled", "false") => w.push("Linux will not be able to start Windows programs, including opening files with Windows apps.".to_string()),
        ("boot.systemd", "true") if wsl_version == 1 => w.push("systemd is not supported on WSL 1 distributions; this setting has no effect there.".to_string()),
        ("boot.systemd", "true") => w.push("Changes how the distribution starts. If it misbehaves, restore the backup.".to_string()),
        ("user.default", _) => w.push("The user must already exist in the distribution. VLINUX cannot check that without running commands as that user's database; if it does not exist, terminals may fail to open.".to_string()),
        _ => {}
    }
    w
}

/// Line diff via longest common subsequence, with 2 lines of context and `Gap` markers.
pub fn diff_lines(old: &str, new: &str) -> Vec<DiffLine> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let (n, m) = (a.len(), b.len());
    // lcs[i][j] = LCS length of a[i..], b[j..]
    let mut lcs = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i] == b[j] { lcs[i + 1][j + 1] + 1 } else { lcs[i + 1][j].max(lcs[i][j + 1]) };
        }
    }
    let mut full: Vec<DiffLine> = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && a[i] == b[j] {
            full.push(DiffLine { kind: DiffKind::Same, text: a[i].to_string() });
            i += 1;
            j += 1;
        } else if j < m && (i == n || lcs[i][j + 1] >= lcs[i + 1][j]) {
            full.push(DiffLine { kind: DiffKind::Add, text: b[j].to_string() });
            j += 1;
        } else {
            full.push(DiffLine { kind: DiffKind::Remove, text: a[i].to_string() });
            i += 1;
        }
    }
    const CTX: usize = 2;
    let changed: Vec<usize> = full.iter().enumerate().filter(|(_, l)| l.kind != DiffKind::Same).map(|(i, _)| i).collect();
    let mut keep = vec![false; full.len()];
    for c in changed {
        for k in c.saturating_sub(CTX)..=(c + CTX).min(full.len().saturating_sub(1)) {
            keep[k] = true;
        }
    }
    let mut out = Vec::new();
    let mut gap = false;
    for (l, k) in full.into_iter().zip(keep) {
        if k {
            out.push(l);
            gap = false;
        } else if !gap {
            out.push(DiffLine { kind: DiffKind::Gap, text: "…".into() });
            gap = true;
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// File access
// ---------------------------------------------------------------------------------------------

/// The one file this module edits. The real implementation goes through the distribution's file
/// share; tests use a temp directory.
pub trait ConfFile {
    fn read(&self) -> Result<Option<String>, ConfError>;
    /// Replace the file atomically (temp file + rename).
    fn write(&self, content: &str) -> Result<(), WriteError>;
    fn remove(&self) -> Result<(), WriteError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteError {
    Denied(String),
    Other(String),
}

pub struct HostConfFile {
    path: PathBuf,
}

impl HostConfFile {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

fn werr(e: &io::Error) -> WriteError {
    if e.kind() == io::ErrorKind::PermissionDenied {
        WriteError::Denied(e.to_string())
    } else {
        WriteError::Other(e.to_string())
    }
}

impl ConfFile for HostConfFile {
    fn read(&self) -> Result<Option<String>, ConfError> {
        match fs::read(&self.path) {
            Ok(b) => {
                if b.len() > MAX_CONF_BYTES {
                    return Err(ConfError::TooLarge);
                }
                String::from_utf8(b).map(Some).map_err(|_| ConfError::Invalid("/etc/wsl.conf is not valid UTF-8 text, so it is not edited.".into()))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(ConfError::Io(format!("Could not read /etc/wsl.conf: {e}"))),
        }
    }

    fn write(&self, content: &str) -> Result<(), WriteError> {
        let dir = self.path.parent().ok_or_else(|| WriteError::Other("no parent folder".into()))?;
        let tmp = dir.join("wsl.conf.vlinux-tmp");
        // Never replace something we did not make.
        if fs::symlink_metadata(&tmp).is_ok() {
            return Err(WriteError::Other("a leftover temporary file (wsl.conf.vlinux-tmp) is in the way; remove it by hand".into()));
        }
        fs::write(&tmp, content).map_err(|e| werr(&e))?;
        if let Err(e) = fs::rename(&tmp, &self.path) {
            let _ = fs::remove_file(&tmp);
            return Err(werr(&e));
        }
        Ok(())
    }

    fn remove(&self) -> Result<(), WriteError> {
        fs::remove_file(&self.path).map_err(|e| werr(&e))
    }
}

// ---------------------------------------------------------------------------------------------
// Backups
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Backup {
    pub id: String,
    pub distro: String,
    pub file: String,
    pub taken_at: u64,
    /// Did the file exist before this change? (If not, restoring removes it.)
    pub existed: bool,
    pub content: String,
    pub original_fp: String,
    /// Fingerprint of what VLINUX wrote after taking this backup. `None` = never applied.
    pub written_fp: Option<String>,
    pub restored_at: Option<u64>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct BackupData {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    backups: Vec<Backup>,
}

pub struct BackupStore {
    path: PathBuf,
}

impl BackupStore {
    pub fn new(dir: &Path) -> Self {
        Self { path: dir.join(BACKUPS_FILE) }
    }

    fn read(&self) -> Result<BackupData, String> {
        match fs::read_to_string(&self.path) {
            Ok(t) => serde_json::from_str(&t).map_err(|e| e.to_string()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(BackupData::default()),
            Err(e) => Err(e.to_string()),
        }
    }

    fn write(&self, data: &BackupData) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_string_pretty(data).map_err(io::Error::other)?)?;
        fs::rename(&tmp, &self.path)
    }

    /// An unreadable store is never overwritten silently: it is set aside and a new one started.
    fn load_for_update(&self, now: u64) -> io::Result<BackupData> {
        match self.read() {
            Ok(d) => Ok(d),
            Err(_) => {
                let aside = self.path.with_file_name(format!("{BACKUPS_FILE}.invalid-{now}"));
                fs::rename(&self.path, aside)?;
                Ok(BackupData::default())
            }
        }
    }

    pub fn list(&self, distro: &str) -> Vec<Backup> {
        let mut v: Vec<Backup> = self.read().map(|d| d.backups).unwrap_or_default().into_iter().filter(|b| b.distro == distro).collect();
        v.sort_by(|a, b| b.taken_at.cmp(&a.taken_at).then_with(|| b.id.cmp(&a.id)));
        v
    }

    pub fn get(&self, distro: &str, id: &str) -> Option<Backup> {
        self.list(distro).into_iter().find(|b| b.id == id)
    }

    fn insert(&self, b: Backup, now: u64) -> io::Result<()> {
        let mut d = self.load_for_update(now)?;
        d.version = 1;
        d.backups.push(b.clone());
        // Keep the newest few per distribution.
        let mut mine: Vec<(u64, String)> = d.backups.iter().filter(|x| x.distro == b.distro).map(|x| (x.taken_at, x.id.clone())).collect();
        mine.sort();
        if mine.len() > KEEP_PER_DISTRO {
            let drop: Vec<String> = mine.iter().take(mine.len() - KEEP_PER_DISTRO).map(|(_, id)| id.clone()).collect();
            d.backups.retain(|x| !(x.distro == b.distro && drop.contains(&x.id)));
        }
        self.write(&d)
    }

    fn update<F: FnOnce(&mut Backup)>(&self, id: &str, now: u64, f: F) -> io::Result<()> {
        let mut d = self.load_for_update(now)?;
        if let Some(b) = d.backups.iter_mut().find(|b| b.id == id) {
            f(b);
        }
        self.write(&d)
    }

    fn discard(&self, id: &str, now: u64) {
        if let Ok(mut d) = self.load_for_update(now) {
            d.backups.retain(|b| b.id != id);
            let _ = self.write(&d);
        }
    }

    fn next_id(&self, distro: &str, now: u64) -> String {
        let used = self.read().map(|d| d.backups.len()).unwrap_or(0);
        format!("{now}-{}-{used}", fingerprint(Some(distro)).chars().take(6).collect::<String>())
    }
}

// ---------------------------------------------------------------------------------------------
// Apply / restore
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ApplyOutcome {
    Applied {
        backup_id: String,
        /// The file was read back and matched what was written.
        verified_on_disk: bool,
        /// Always `false`: VLINUX cannot see whether the distribution is using the new values.
        verified_active: bool,
        message: String,
    },
    /// The distribution's file is not writable by the account VLINUX runs as. Nothing was changed.
    NotWritable { reason: String, proposed: String, instructions: String },
}

fn manual_instructions(proposed_len: usize) -> String {
    format!(
        "VLINUX does not use administrator or root access. To make this change yourself, open a terminal in this distribution and edit /etc/wsl.conf with sudo (for example `sudo nano /etc/wsl.conf`), pasting the proposed content ({proposed_len} bytes) shown here. Then restart the distribution."
    )
}

/// Write `new_text` after taking a backup. `expected_fp` is the fingerprint the preview was based on.
pub fn apply(file: &dyn ConfFile, store: &BackupStore, distro: &str, expected_fp: &str, new_text: &str, now: u64) -> Result<ApplyOutcome, ConfError> {
    if new_text.len() > MAX_CONF_BYTES {
        return Err(ConfError::TooLarge);
    }
    let check = IniDoc::parse(new_text);
    if check.is_malformed() {
        return Err(ConfError::Malformed(check.malformed));
    }
    let current = file.read()?;
    if fingerprint(current.as_deref()) != expected_fp {
        return Err(ConfError::Changed);
    }
    if current.as_deref() == Some(new_text) {
        return Err(ConfError::NoChange);
    }
    if let Some(c) = &current {
        if IniDoc::parse(c).is_malformed() {
            return Err(ConfError::Malformed(IniDoc::parse(c).malformed));
        }
    }
    let backup = Backup {
        id: store.next_id(distro, now),
        distro: distro.to_string(),
        file: CONF_PATH.into(),
        taken_at: now,
        existed: current.is_some(),
        content: current.clone().unwrap_or_default(),
        original_fp: fingerprint(current.as_deref()),
        written_fp: None,
        restored_at: None,
    };
    store.insert(backup.clone(), now).map_err(|e| ConfError::BackupFailed(e.to_string()))?;

    if let Err(e) = file.write(new_text) {
        // Nothing was replaced, so the backup would only be noise.
        store.discard(&backup.id, now);
        return match e {
            WriteError::Denied(reason) => Ok(ApplyOutcome::NotWritable { reason, proposed: new_text.to_string(), instructions: manual_instructions(new_text.len()) }),
            WriteError::Other(m) => Err(ConfError::Io(format!("Could not write /etc/wsl.conf: {m}. Nothing was changed."))),
        };
    }

    let after = file.read();
    if after.as_ref().ok().and_then(|a| a.as_deref()) != Some(new_text) {
        let detail = match &after {
            Ok(_) => "the content differs".to_string(),
            Err(e) => e.to_string(),
        };
        let restored = put_back(file, &backup).is_ok();
        return Err(ConfError::VerifyFailed { restored, detail });
    }
    let fp = fingerprint(Some(new_text));
    store.update(&backup.id, now, |b| b.written_fp = Some(fp.clone())).map_err(|e| ConfError::BackupFailed(format!("the file was written but its backup record could not be updated: {e}")))?;
    Ok(ApplyOutcome::Applied {
        backup_id: backup.id,
        verified_on_disk: true,
        verified_active: false,
        message: format!("Saved to /etc/wsl.conf and read back successfully. {RESTART_NOTE}"),
    })
}

fn put_back(file: &dyn ConfFile, b: &Backup) -> Result<(), WriteError> {
    if b.existed {
        file.write(&b.content)
    } else {
        file.remove()
    }
}

/// Put a backup's content back. The current state is backed up first, so this can be undone too.
pub fn restore(file: &dyn ConfFile, store: &BackupStore, distro: &str, id: &str, force: bool, now: u64) -> Result<ApplyOutcome, ConfError> {
    let b = store.get(distro, id).ok_or_else(|| ConfError::NotFound("That backup no longer exists.".into()))?;
    let current = file.read()?;
    let cur_fp = fingerprint(current.as_deref());
    match &b.written_fp {
        None => return Err(ConfError::Invalid("That backup was never applied, so there is nothing to undo.".into())),
        Some(w) if *w != cur_fp && !force => return Err(ConfError::Diverged),
        _ => {}
    }
    let target_fp = fingerprint(if b.existed { Some(b.content.as_str()) } else { None });
    if cur_fp == target_fp {
        return Err(ConfError::NoChange);
    }
    let safety = Backup {
        id: store.next_id(distro, now),
        distro: distro.to_string(),
        file: CONF_PATH.into(),
        taken_at: now,
        existed: current.is_some(),
        content: current.clone().unwrap_or_default(),
        original_fp: cur_fp,
        written_fp: Some(target_fp.clone()),
        restored_at: None,
    };
    store.insert(safety.clone(), now).map_err(|e| ConfError::BackupFailed(e.to_string()))?;
    if let Err(e) = put_back(file, &b) {
        store.discard(&safety.id, now);
        return match e {
            WriteError::Denied(reason) => Ok(ApplyOutcome::NotWritable {
                reason,
                proposed: if b.existed { b.content.clone() } else { String::new() },
                instructions: manual_instructions(b.content.len()),
            }),
            WriteError::Other(m) => Err(ConfError::Io(format!("Could not restore /etc/wsl.conf: {m}. Nothing was changed."))),
        };
    }
    let after = file.read()?;
    if fingerprint(after.as_deref()) != target_fp {
        return Err(ConfError::VerifyFailed { restored: false, detail: "the restored file did not read back as expected".into() });
    }
    let _ = store.update(&b.id, now, |x| x.restored_at = Some(now));
    Ok(ApplyOutcome::Applied {
        backup_id: safety.id,
        verified_on_disk: true,
        verified_active: false,
        message: format!("Restored. The state before the restore was saved as a new backup, so this can be undone. {RESTART_NOTE}"),
    })
}

// ---------------------------------------------------------------------------------------------
// Access rule
// ---------------------------------------------------------------------------------------------

/// Reading or writing `/etc/wsl.conf` goes through the distribution's file share, which starts a
/// stopped distribution. VLINUX does that only on an explicit Start, so config access requires the
/// distribution to be running already.
pub fn check_accessible(report: &WslReport, name: &str) -> Result<u8, ConfError> {
    if !valid_distro_name(name) {
        return Err(ConfError::Invalid(format!("'{}' is not a valid distribution name.", name.chars().take(40).collect::<String>())));
    }
    if report.status != RuntimeStatus::Ready {
        return Err(ConfError::Invalid("WSL is not ready. Check the setup status first.".into()));
    }
    let d = report.distros.iter().find(|d| d.name == name).ok_or_else(|| ConfError::NotFound(format!("No distribution named '{name}' is registered with WSL.")))?;
    if d.kind == DistroKind::Internal {
        return Err(ConfError::Invalid(format!("'{name}' belongs to another application and is never touched by VLINUX.")));
    }
    if d.state != DistroState::Running {
        return Err(ConfError::NotRunning(name.to_string()));
    }
    Ok(d.wsl_version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::testutil::TempDir;
    use crate::model::DistroKind::{Internal, Regular};
    use crate::model::DistroState::{Running, Stopped};
    use crate::testing::fake_report;

    const SAMPLE: &str = "# my distro\n[boot]\nsystemd=true\n\n[automount]\nenabled = true # keep\nOptions = \"metadata,umask=22\"\n\n[custom]\nthing=1\n";

    fn ch(id: &str, v: Option<&str>) -> Change {
        Change { id: id.into(), value: v.map(|s| s.to_string()) }
    }

    #[test]
    fn parse_render_roundtrip_is_exact() {
        for t in [SAMPLE, "", "[a]\nb=c", "[a]\r\nb=c\r\n", "\n\n[x]\n", "# only a comment\n"] {
            assert_eq!(IniDoc::parse(t).render(), t, "{t:?}");
        }
    }

    #[test]
    fn lookup_is_case_insensitive_and_unquotes() {
        let d = IniDoc::parse(SAMPLE);
        assert_eq!(d.get("AUTOMOUNT", "ENABLED").as_deref(), Some("true"));
        assert_eq!(d.get("automount", "options").as_deref(), Some("metadata,umask=22"));
        assert_eq!(d.get("boot", "systemd").as_deref(), Some("true"));
        assert_eq!(d.get("boot", "nothing"), None);
        assert_eq!(d.get("nosuch", "enabled"), None);
    }

    #[test]
    fn edits_preserve_everything_else() {
        let p = plan("D", Some(SAMPLE), &[ch("automount.enabled", Some("false"))], 2).unwrap();
        assert_eq!(p.new_text, SAMPLE.replace("enabled = true # keep", "enabled = false # keep"));
        // Unknown keys, unknown sections, comments, odd spacing and quotes all survive.
        assert!(p.new_text.contains("# my distro") && p.new_text.contains("[custom]") && p.new_text.contains("Options = \"metadata,umask=22\""));
        assert_eq!(p.diff.iter().filter(|l| l.kind == DiffKind::Add).count(), 1);
        assert_eq!(p.diff.iter().filter(|l| l.kind == DiffKind::Remove).count(), 1);
        assert_eq!(p.based_on, fingerprint(Some(SAMPLE)));
    }

    #[test]
    fn missing_keys_and_sections_are_added_in_the_right_place() {
        let p = plan("D", Some(SAMPLE), &[ch("interop.enabled", Some("false")), ch("boot.systemd", Some("false"))], 2).unwrap();
        let d = IniDoc::parse(&p.new_text);
        assert_eq!(d.get("interop", "enabled").as_deref(), Some("false"));
        assert_eq!(d.get("boot", "systemd").as_deref(), Some("false"));
        assert!(!d.is_malformed());
        // A new key goes into an existing section.
        let q = plan("D", Some(SAMPLE), &[ch("automount.root", Some("/mnt/"))], 2).unwrap();
        let lines: Vec<&str> = q.new_text.lines().collect();
        let pos = lines.iter().position(|l| *l == "root=/mnt/").unwrap();
        assert!(lines[..pos].iter().rev().any(|l| l.starts_with("Options")) && lines[pos + 1].is_empty());
    }

    #[test]
    fn creating_the_file_from_nothing() {
        let p = plan("D", None, &[ch("user.default", Some("dana"))], 2).unwrap();
        assert!(p.creates_file);
        assert_eq!(p.new_text, "[user]\ndefault=dana\n");
        assert_eq!(p.based_on, fingerprint(None));
        assert_ne!(fingerprint(None), fingerprint(Some("")));
    }

    #[test]
    fn crlf_files_stay_crlf() {
        let p = plan("D", Some("[boot]\r\nsystemd=false\r\n"), &[ch("boot.systemd", Some("true")), ch("interop.enabled", Some("false"))], 2).unwrap();
        assert_eq!(p.new_text, "[boot]\r\nsystemd=true\r\n\r\n[interop]\r\nenabled=false\r\n");
        assert!(!p.new_text.replace("\r\n", "").contains('\n'));
    }

    #[test]
    fn unset_removes_only_that_line() {
        let p = plan("D", Some(SAMPLE), &[ch("boot.systemd", None)], 2).unwrap();
        assert!(!p.new_text.contains("systemd") && p.new_text.contains("[boot]") && p.new_text.contains("thing=1"));
        assert!(plan("D", Some(SAMPLE), &[ch("user.default", None)], 2).is_err_and(|e| e == ConfError::NoChange));
    }

    #[test]
    fn invalid_requests_are_rejected() {
        for (id, v) in [("automount.enabled", "yes"), ("automount.root", "relative"), ("automount.root", "/"), ("automount.root", "/mnt/../etc"), ("automount.root", "/a b"), ("user.default", "Root User"), ("user.default", "a\nb"), ("user.default", "dana\n[boot]")] {
            assert!(matches!(plan("D", Some(SAMPLE), &[ch(id, Some(v))], 2), Err(ConfError::Invalid(_))), "{id}={v:?}");
        }
        assert!(matches!(plan("D", Some(SAMPLE), &[ch("boot.command", Some("rm -rf /"))], 2), Err(ConfError::Invalid(_))));
        assert!(matches!(plan("D", Some(SAMPLE), &[ch("boot.systemd", Some("true")), ch("boot.systemd", Some("false"))], 2), Err(ConfError::Invalid(_))));
        assert!(matches!(plan("D", Some(SAMPLE), &[], 2), Err(ConfError::NoChange)));
        assert!(matches!(plan("D", Some(SAMPLE), &[ch("boot.systemd", Some("true"))], 2), Err(ConfError::NoChange)));
    }

    #[test]
    fn malformed_files_are_not_edited() {
        for bad in ["[boot\nsystemd=true\n", "systemd=true\n", "[boot]\nthis is not a pair\n"] {
            assert!(IniDoc::parse(bad).is_malformed(), "{bad:?}");
            assert!(matches!(plan("D", Some(bad), &[ch("boot.systemd", Some("false"))], 2), Err(ConfError::Malformed(_))), "{bad:?}");
            assert!(!view("D", Some(bad)).editable);
        }
        // The readable part of a malformed file is still shown.
        let v = view("D", Some("[boot]\nsystemd=true\noops\n"));
        assert_eq!(v.malformed, vec![3]);
        assert_eq!(v.settings.iter().find(|s| s.id == "boot.systemd").unwrap().value.as_deref(), Some("true"));
    }

    #[test]
    fn view_lists_unmanaged_keys_and_states() {
        let v = view("D", Some(SAMPLE));
        assert!(v.exists && v.editable);
        assert!(v.other_keys.contains(&"custom.thing".to_string()) && v.other_keys.contains(&"automount.Options".to_string()));
        assert_eq!(v.settings.iter().find(|s| s.id == "interop.enabled").unwrap().value, None);
        let none = view("D", None);
        assert!(!none.exists && none.editable && none.notes.iter().any(|n| n.contains("no /etc/wsl.conf")));
    }

    #[test]
    fn warnings_explain_consequences() {
        let p = plan("D", Some(SAMPLE), &[ch("automount.enabled", Some("false")), ch("automount.root", Some("/win/"))], 2).unwrap();
        assert!(p.warnings.iter().any(|w| w.contains("shared-folder")));
        assert!(p.restart_note.contains("restart") && p.restart_note.contains("will not restart"));
        let q = plan("D", None, &[ch("boot.systemd", Some("true"))], 1).unwrap();
        assert!(q.warnings.iter().any(|w| w.contains("WSL 1")));
    }

    #[test]
    fn diff_has_context_and_gaps() {
        let old: String = (0..20).map(|i| format!("l{i}\n")).collect();
        let new = old.replace("l10\n", "x\n").replace("l1\n", "y\n");
        let d = diff_lines(&old, &new);
        assert!(d.iter().any(|l| l.kind == DiffKind::Gap));
        assert!(d.iter().any(|l| l.kind == DiffKind::Add && l.text == "x"));
        assert!(d.iter().any(|l| l.kind == DiffKind::Remove && l.text == "l10"));
        assert!(d.len() < 20);
        assert!(diff_lines("a\n", "a\n").iter().all(|l| l.kind == DiffKind::Gap) || diff_lines("a\n", "a\n").is_empty());
    }

    // --- apply / restore against a real temp directory ------------------------------------

    struct Env {
        dir: TempDir,
        conf: PathBuf,
        file: HostConfFile,
        store: BackupStore,
    }

    fn env(initial: Option<&str>) -> Env {
        let dir = TempDir::new("wslconf");
        let etc = dir.join("etc");
        fs::create_dir_all(&etc).unwrap();
        let conf = etc.join("wsl.conf");
        if let Some(t) = initial {
            fs::write(&conf, t).unwrap();
        }
        let store = BackupStore::new(&dir.join("appdata"));
        Env { file: HostConfFile::new(conf.clone()), conf, store, dir }
    }

    fn apply_change(e: &Env, current: Option<&str>, changes: &[Change], now: u64) -> Result<ApplyOutcome, ConfError> {
        let p = plan("D", current, changes, 2)?;
        apply(&e.file, &e.store, "D", &p.based_on, &p.new_text, now)
    }

    #[test]
    fn apply_backs_up_writes_verifies_and_never_claims_activity() {
        let e = env(Some(SAMPLE));
        let out = apply_change(&e, Some(SAMPLE), &[ch("automount.enabled", Some("false"))], 100).unwrap();
        let ApplyOutcome::Applied { backup_id, verified_on_disk, verified_active, message } = out else { panic!("expected Applied") };
        assert!(verified_on_disk && !verified_active);
        assert!(message.contains("restart") && !message.to_lowercase().contains("is active"));
        assert!(fs::read_to_string(&e.conf).unwrap().contains("enabled = false # keep"));
        let b = e.store.get("D", &backup_id).unwrap();
        assert_eq!(b.content, SAMPLE);
        assert!(b.existed && b.written_fp.is_some());
        assert!(!e.conf.with_file_name("wsl.conf.vlinux-tmp").exists());
    }

    #[test]
    fn stale_preview_is_refused_and_nothing_is_written() {
        let e = env(Some(SAMPLE));
        let p = plan("D", Some(SAMPLE), &[ch("boot.systemd", Some("false"))], 2).unwrap();
        fs::write(&e.conf, "[user]\ndefault=someone\n").unwrap();
        assert_eq!(apply(&e.file, &e.store, "D", &p.based_on, &p.new_text, 1), Err(ConfError::Changed));
        assert_eq!(fs::read_to_string(&e.conf).unwrap(), "[user]\ndefault=someone\n");
        assert!(e.store.list("D").is_empty());
    }

    #[test]
    fn restore_returns_the_original_and_is_itself_reversible() {
        let e = env(Some(SAMPLE));
        let ApplyOutcome::Applied { backup_id, .. } = apply_change(&e, Some(SAMPLE), &[ch("boot.systemd", Some("false"))], 100).unwrap() else { panic!() };
        let changed = fs::read_to_string(&e.conf).unwrap();
        let ApplyOutcome::Applied { backup_id: undo_id, .. } = restore(&e.file, &e.store, "D", &backup_id, false, 200).unwrap() else { panic!() };
        assert_eq!(fs::read_to_string(&e.conf).unwrap(), SAMPLE);
        // And the restore can be undone.
        restore(&e.file, &e.store, "D", &undo_id, false, 300).unwrap();
        assert_eq!(fs::read_to_string(&e.conf).unwrap(), changed);
        assert!(e.store.get("D", &backup_id).unwrap().restored_at.is_some());
    }

    #[test]
    fn restore_refuses_to_discard_someone_elses_edit_unless_forced() {
        let e = env(Some(SAMPLE));
        let ApplyOutcome::Applied { backup_id, .. } = apply_change(&e, Some(SAMPLE), &[ch("boot.systemd", Some("false"))], 100).unwrap() else { panic!() };
        fs::write(&e.conf, "# edited by hand\n[boot]\nsystemd=true\n").unwrap();
        assert_eq!(restore(&e.file, &e.store, "D", &backup_id, false, 200), Err(ConfError::Diverged));
        assert_eq!(fs::read_to_string(&e.conf).unwrap(), "# edited by hand\n[boot]\nsystemd=true\n");
        restore(&e.file, &e.store, "D", &backup_id, true, 300).unwrap();
        assert_eq!(fs::read_to_string(&e.conf).unwrap(), SAMPLE);
        // The hand edit was saved before being replaced.
        assert!(e.store.list("D").iter().any(|b| b.content == "# edited by hand\n[boot]\nsystemd=true\n"));
    }

    #[test]
    fn restoring_a_created_file_removes_it() {
        let e = env(None);
        let ApplyOutcome::Applied { backup_id, .. } = apply_change(&e, None, &[ch("user.default", Some("dana"))], 100).unwrap() else { panic!() };
        assert!(e.conf.exists());
        restore(&e.file, &e.store, "D", &backup_id, false, 200).unwrap();
        assert!(!e.conf.exists());
    }

    #[test]
    fn restore_of_unknown_other_distro_or_unapplied_backups_fails() {
        let e = env(Some(SAMPLE));
        assert!(matches!(restore(&e.file, &e.store, "D", "nope", false, 1), Err(ConfError::NotFound(_))));
        let ApplyOutcome::Applied { backup_id, .. } = apply_change(&e, Some(SAMPLE), &[ch("boot.systemd", Some("false"))], 100).unwrap() else { panic!() };
        // Backups are per distribution.
        assert!(matches!(restore(&e.file, &e.store, "Other", &backup_id, true, 1), Err(ConfError::NotFound(_))));
    }

    struct DeniedFile(String);
    impl ConfFile for DeniedFile {
        fn read(&self) -> Result<Option<String>, ConfError> {
            Ok(Some(self.0.clone()))
        }
        fn write(&self, _: &str) -> Result<(), WriteError> {
            Err(WriteError::Denied("Access is denied. (os error 5)".into()))
        }
        fn remove(&self) -> Result<(), WriteError> {
            Err(WriteError::Denied("denied".into()))
        }
    }

    #[test]
    fn permission_denied_yields_manual_instructions_not_elevation() {
        let dir = TempDir::new("wslconf-denied");
        let store = BackupStore::new(dir.path());
        let f = DeniedFile(SAMPLE.to_string());
        let p = plan("D", Some(SAMPLE), &[ch("boot.systemd", Some("false"))], 2).unwrap();
        let out = apply(&f, &store, "D", &p.based_on, &p.new_text, 5).unwrap();
        let ApplyOutcome::NotWritable { proposed, instructions, .. } = out else { panic!("expected NotWritable") };
        assert_eq!(proposed, p.new_text);
        assert!(instructions.contains("sudo") && instructions.contains("does not use administrator"));
        // The pointless backup was dropped.
        assert!(store.list("D").is_empty());
    }

    #[test]
    fn a_write_that_does_not_read_back_is_rolled_back() {
        let e = env(Some(SAMPLE));
        struct Flaky<'a> {
            inner: &'a HostConfFile,
            writes: std::cell::Cell<u32>,
        }
        impl ConfFile for Flaky<'_> {
            fn read(&self) -> Result<Option<String>, ConfError> {
                self.inner.read()
            }
            fn write(&self, c: &str) -> Result<(), WriteError> {
                // The first write is corrupted; the rollback write is honest.
                let n = self.writes.get();
                self.writes.set(n + 1);
                if n == 0 {
                    self.inner.write(&format!("{c}# garbage"))
                } else {
                    self.inner.write(c)
                }
            }
            fn remove(&self) -> Result<(), WriteError> {
                self.inner.remove()
            }
        }
        let f = Flaky { inner: &e.file, writes: std::cell::Cell::new(0) };
        let p = plan("D", Some(SAMPLE), &[ch("boot.systemd", Some("false"))], 2).unwrap();
        let err = apply(&f, &e.store, "D", &p.based_on, &p.new_text, 9).unwrap_err();
        assert!(matches!(err, ConfError::VerifyFailed { restored: true, .. }));
        assert_eq!(fs::read_to_string(&e.conf).unwrap(), SAMPLE);
    }

    #[test]
    fn malformed_current_file_is_never_overwritten() {
        let bad = "[boot\nsystemd=true\n";
        let e = env(Some(bad));
        let r = apply(&e.file, &e.store, "D", &fingerprint(Some(bad)), "[boot]\nsystemd=false\n", 1);
        assert!(matches!(r, Err(ConfError::Malformed(_))));
        assert_eq!(fs::read_to_string(&e.conf).unwrap(), bad);
    }

    #[test]
    fn oversized_and_non_utf8_files_are_refused() {
        let e = env(None);
        fs::write(&e.conf, vec![b'#'; MAX_CONF_BYTES + 1]).unwrap();
        assert_eq!(e.file.read(), Err(ConfError::TooLarge));
        fs::write(&e.conf, [0xff, 0xfe, 0x00]).unwrap();
        assert!(matches!(e.file.read(), Err(ConfError::Invalid(_))));
        let big = "#".repeat(MAX_CONF_BYTES + 1);
        assert_eq!(plan("D", Some(&big), &[ch("boot.systemd", Some("true"))], 2), Err(ConfError::TooLarge));
    }

    #[test]
    fn a_leftover_temp_file_is_not_overwritten() {
        let e = env(Some(SAMPLE));
        fs::write(e.conf.with_file_name("wsl.conf.vlinux-tmp"), "someone's file").unwrap();
        let r = apply_change(&e, Some(SAMPLE), &[ch("boot.systemd", Some("false"))], 1);
        assert!(matches!(r, Err(ConfError::Io(_))));
        assert_eq!(fs::read_to_string(e.conf.with_file_name("wsl.conf.vlinux-tmp")).unwrap(), "someone's file");
        assert_eq!(fs::read_to_string(&e.conf).unwrap(), SAMPLE);
    }

    #[test]
    fn backups_are_capped_per_distro() {
        let e = env(Some("[boot]\nsystemd=false\n"));
        let mut cur = "[boot]\nsystemd=false\n".to_string();
        for i in 0..(KEEP_PER_DISTRO as u64 + 5) {
            let v = if i % 2 == 0 { "true" } else { "false" };
            apply_change(&e, Some(&cur), &[ch("boot.systemd", Some(v))], 1000 + i).unwrap();
            cur = fs::read_to_string(&e.conf).unwrap();
        }
        assert_eq!(e.store.list("D").len(), KEEP_PER_DISTRO);
        let _ = &e.dir;
    }

    #[test]
    fn corrupt_backup_store_is_set_aside_not_overwritten() {
        let e = env(Some(SAMPLE));
        let sp = e.dir.join("appdata");
        fs::create_dir_all(&sp).unwrap();
        fs::write(sp.join(BACKUPS_FILE), "not json").unwrap();
        apply_change(&e, Some(SAMPLE), &[ch("boot.systemd", Some("false"))], 77).unwrap();
        assert!(fs::read_dir(&sp).unwrap().any(|f| f.unwrap().file_name().to_string_lossy().contains(".invalid-77")));
    }

    #[test]
    fn config_access_needs_a_running_regular_distro() {
        let r = fake_report(&[("Run", Running, Regular, 2), ("Stop", Stopped, Regular, 2), ("docker-desktop", Running, Internal, 2)], None);
        assert_eq!(check_accessible(&r, "Run"), Ok(2));
        assert_eq!(check_accessible(&r, "Stop"), Err(ConfError::NotRunning("Stop".into())));
        assert!(matches!(check_accessible(&r, "docker-desktop"), Err(ConfError::Invalid(_))));
        assert!(matches!(check_accessible(&r, "Nope"), Err(ConfError::NotFound(_))));
        assert!(matches!(check_accessible(&r, "../x"), Err(ConfError::Invalid(_))));
    }
}
