//! Record of the distributions VLINUX itself installed (with the user's approval).
//!
//! This is what lets the UI tell "Installed by VLINUX" from "Existing". It is VLINUX's own state
//! file (JSON in the app config directory), never part of a distribution. A corrupt file is moved
//! aside, not deleted. Nothing here ever touches a distribution.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const MANAGED_FILE: &str = "distros.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedEntry {
    pub name: String,
    pub installed_at: u64,
    /// The official WSL name the install used.
    pub source: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct FileData {
    version: u32,
    installed_by_vlinux: Vec<ManagedEntry>,
}

pub struct ManagedStore {
    path: PathBuf,
}

impl ManagedStore {
    pub fn new(dir: &Path) -> Self {
        Self { path: dir.join(MANAGED_FILE) }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn read(&self) -> Result<FileData, String> {
        match fs::read_to_string(&self.path) {
            Ok(t) => serde_json::from_str::<FileData>(&t).map_err(|e| e.to_string()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(FileData::default()),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Registered names VLINUX installed. Unreadable/corrupt file = nobody is claimed as ours.
    pub fn names(&self) -> HashSet<String> {
        self.read().map(|d| d.installed_by_vlinux.into_iter().map(|e| e.name).collect()).unwrap_or_default()
    }

    pub fn entries(&self) -> Vec<ManagedEntry> {
        self.read().map(|d| d.installed_by_vlinux).unwrap_or_default()
    }

    /// Remember that VLINUX installed `name`. Idempotent (case-insensitive).
    pub fn record(&self, name: &str, source: &str, now: u64) -> io::Result<()> {
        let mut data = match self.read() {
            Ok(d) => d,
            Err(_) => {
                // Keep the unreadable file for the user; start a fresh record.
                let aside = self.path.with_file_name(format!("{MANAGED_FILE}.invalid-{now}"));
                fs::rename(&self.path, aside)?;
                FileData::default()
            }
        };
        if data.installed_by_vlinux.iter().any(|e| e.name.eq_ignore_ascii_case(name)) {
            return Ok(());
        }
        data.version = 1;
        data.installed_by_vlinux.push(ManagedEntry { name: name.to_string(), installed_at: now, source: source.to_string() });
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(&data).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        fs::write(&tmp, text)?;
        fs::rename(&tmp, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "vlinux-managed-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn missing_file_claims_nothing() {
        let d = temp_dir("missing");
        assert!(ManagedStore::new(&d).names().is_empty());
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn records_roundtrip_and_is_idempotent() {
        let d = temp_dir("rt");
        let s = ManagedStore::new(&d);
        s.record("Debian", "Debian", 100).unwrap();
        s.record("debian", "Debian", 200).unwrap();
        s.record("Ubuntu-24.04", "Ubuntu-24.04", 300).unwrap();
        let names = s.names();
        assert_eq!(names.len(), 2);
        assert!(names.contains("Debian") && names.contains("Ubuntu-24.04"));
        assert_eq!(s.entries()[0].installed_at, 100);
        assert!(!d.join("distros.json.tmp").exists());
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn corrupt_file_is_kept_not_deleted_and_claims_nothing() {
        let d = temp_dir("corrupt");
        fs::write(d.join(MANAGED_FILE), "{ not json").unwrap();
        let s = ManagedStore::new(&d);
        assert!(s.names().is_empty());
        s.record("Debian", "Debian", 42).unwrap();
        assert_eq!(fs::read_to_string(d.join("distros.json.invalid-42")).unwrap(), "{ not json");
        assert!(s.names().contains("Debian"));
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn unknown_fields_are_tolerated() {
        let d = temp_dir("unknown");
        fs::write(d.join(MANAGED_FILE), r#"{"version":9,"extra":1,"installed_by_vlinux":[{"name":"X","installed_at":1,"source":"X"}]}"#).unwrap();
        assert!(ManagedStore::new(&d).names().contains("X"));
        let _ = fs::remove_dir_all(d);
    }
}
