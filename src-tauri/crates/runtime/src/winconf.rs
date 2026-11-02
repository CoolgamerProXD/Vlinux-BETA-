//! The Windows-side `%USERPROFILE%\.wslconfig`, **read-only** (M5).
//!
//! This file configures the whole WSL 2 virtual machine for every distribution, and a change needs
//! `wsl --shutdown`, which VLINUX never runs. Writing it is a separate decision that has not been
//! made, so this module only reads and describes it. Nothing here can write or create the file.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::distroconf::IniDoc;

const MAX_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WinEntry {
    pub section: String,
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WinResource {
    pub label: String,
    /// The value in `.wslconfig`, or `None` = not set (WSL's default applies).
    pub value: Option<String>,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WindowsConfig {
    /// Always the symbolic path; the real user folder name is never sent to the UI.
    pub display_path: String,
    pub exists: bool,
    pub malformed: Vec<usize>,
    pub resources: Vec<WinResource>,
    pub entries: Vec<WinEntry>,
    /// Logical processors of this PC (what WSL 2 can use at most).
    pub host_processors: Option<usize>,
    pub notes: Vec<String>,
    pub read_error: Option<String>,
}

pub fn default_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".wslconfig"))
}

const RESOURCE_KEYS: [(&str, &str, &str); 4] = [
    ("memory", "Memory limit for WSL 2", "Not set: WSL 2 decides (a share of this PC's memory)."),
    ("processors", "Processor limit for WSL 2", "Not set: WSL 2 may use all logical processors."),
    ("swap", "Swap size", "Not set: WSL's default swap applies."),
    ("localhostForwarding", "Localhost forwarding", "Not set: WSL's default applies."),
];

/// Read `.wslconfig` from `path` (injectable for tests). A missing file is normal.
pub fn read(path: Option<&Path>, host_processors: Option<usize>) -> WindowsConfig {
    let mut cfg = WindowsConfig {
        display_path: "%USERPROFILE%\\.wslconfig".into(),
        exists: false,
        malformed: Vec::new(),
        resources: Vec::new(),
        entries: Vec::new(),
        host_processors,
        notes: vec![
            "This file is shared by every distribution and is owned by Windows, not by a distribution.".into(),
            "VLINUX only reads it. Changing it needs the whole WSL VM to restart (`wsl --shutdown`), which VLINUX never does, so edit it yourself if you want to change it.".into(),
        ],
        read_error: None,
    };
    let text = match path {
        None => {
            cfg.read_error = Some("The Windows user profile folder could not be determined.".into());
            None
        }
        Some(p) => match fs::metadata(p) {
            Ok(m) if m.len() > MAX_BYTES => {
                cfg.exists = true;
                cfg.read_error = Some("The file is too large to display.".into());
                None
            }
            Ok(_) => match fs::read(p) {
                Ok(b) => {
                    cfg.exists = true;
                    match String::from_utf8(b) {
                        Ok(t) => Some(t),
                        Err(_) => {
                            cfg.read_error = Some("The file is not valid UTF-8 text.".into());
                            None
                        }
                    }
                }
                Err(e) => {
                    cfg.read_error = Some(format!("Could not read the file: {e}"));
                    None
                }
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                cfg.notes.push("There is no .wslconfig, so WSL's defaults apply.".into());
                None
            }
            Err(e) => {
                cfg.read_error = Some(format!("Could not read the file: {e}"));
                None
            }
        },
    };
    let doc = IniDoc::parse(text.as_deref().unwrap_or(""));
    cfg.malformed = doc.malformed.clone();
    cfg.resources = RESOURCE_KEYS
        .iter()
        .map(|(k, label, note)| WinResource { label: (*label).into(), value: doc.get("wsl2", k), note: (*note).into() })
        .collect();
    cfg.entries = doc
        .keys()
        .into_iter()
        .take(100)
        .filter_map(|(s, k)| doc.get(&s, &k).map(|v| WinEntry { section: s, key: k, value: v.chars().take(200).collect() }))
        .collect();
    if doc.is_malformed() {
        cfg.notes.push("Some lines could not be understood; the entries shown are the ones that could be read.".into());
    }
    cfg
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::testutil::TempDir;

    #[test]
    fn missing_file_is_normal() {
        let d = TempDir::new("winconf");
        let c = read(Some(&d.join(".wslconfig")), Some(8));
        assert!(!c.exists && c.read_error.is_none());
        assert!(c.resources.iter().all(|r| r.value.is_none()));
        assert!(c.notes.iter().any(|n| n.contains("no .wslconfig")));
        assert_eq!(c.host_processors, Some(8));
        assert_eq!(c.display_path, "%USERPROFILE%\\.wslconfig");
    }

    #[test]
    fn reads_resources_and_entries_without_writing() {
        let d = TempDir::new("winconf");
        let p = d.join(".wslconfig");
        let text = "[wsl2]\nmemory=8GB\nprocessors=4\nkernel=C:\\\\x\\\\k\n[experimental]\nautoMemoryReclaim=gradual\n";
        fs::write(&p, text).unwrap();
        let c = read(Some(&p), None);
        assert!(c.exists);
        let get = |l: &str| c.resources.iter().find(|r| r.label.starts_with(l)).unwrap().value.clone();
        assert_eq!(get("Memory").as_deref(), Some("8GB"));
        assert_eq!(get("Processor").as_deref(), Some("4"));
        assert_eq!(get("Swap"), None);
        assert_eq!(c.entries.len(), 4);
        assert_eq!(fs::read_to_string(&p).unwrap(), text);
    }

    #[test]
    fn malformed_and_unreadable_files_are_described_not_fixed() {
        let d = TempDir::new("winconf");
        let p = d.join(".wslconfig");
        fs::write(&p, "[wsl2]\nmemory=4GB\nnot a pair\n").unwrap();
        let c = read(Some(&p), None);
        assert_eq!(c.malformed, vec![3]);
        assert_eq!(c.resources[0].value.as_deref(), Some("4GB"));
        fs::write(&p, [0xffu8, 0xfe, 0x00, 0x61]).unwrap();
        assert!(read(Some(&p), None).read_error.is_some());
        assert!(read(None, None).read_error.is_some());
        fs::write(&p, vec![b'#'; (MAX_BYTES + 1) as usize]).unwrap();
        assert!(read(Some(&p), None).read_error.is_some());
    }
}
