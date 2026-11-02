//! VLINUX's own record of the mappings it applied (`mappings.json` in the app config directory,
//! never inside a distribution). It is what makes a mapping *reversible*: it remembers what was
//! replaced (nothing, an empty folder, a link, or a folder that was moved aside and where to).
//! A corrupt file is moved aside, not deleted.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const MAPPINGS_FILE: &str = "mappings.json";

/// What was at the Linux path before VLINUX put the link there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Replaced {
    /// Nothing existed.
    Nothing,
    /// An empty folder (removed with `rmdir`, which cannot remove anything non-empty).
    EmptyDir,
    /// A folder or file, renamed to `backup` (a Linux path). Nothing was deleted or copied.
    MovedAside { backup: String },
    /// A different symbolic link (only the link was replaced).
    Link { old_target: String },
    /// The link already existed and was only recorded.
    Adopted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MappingRecord {
    pub distro: String,
    pub folder: String,
    /// The link, e.g. `/home/me/Downloads`.
    pub linux_path: String,
    /// What it points to inside the distribution, e.g. `/mnt/d/Media/Downloads`.
    pub target: String,
    /// The Windows folder behind it.
    pub windows_path: String,
    pub applied_at: u64,
    pub replaced: Replaced,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct FileData {
    version: u32,
    mappings: Vec<MappingRecord>,
}

pub struct MappingStore {
    path: PathBuf,
}

impl MappingStore {
    pub fn new(dir: &Path) -> Self {
        Self { path: dir.join(MAPPINGS_FILE) }
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

    fn write(&self, data: &FileData) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(data).map_err(io::Error::other)?;
        fs::write(&tmp, text)?;
        fs::rename(&tmp, &self.path)
    }

    /// Unreadable/corrupt = no mapping is claimed (the file is kept for the user).
    pub fn all(&self) -> Vec<MappingRecord> {
        self.read().map(|d| d.mappings).unwrap_or_default()
    }

    pub fn for_distro(&self, distro: &str) -> Vec<MappingRecord> {
        self.all().into_iter().filter(|r| r.distro == distro).collect()
    }

    pub fn find(&self, distro: &str, folder: &str) -> Option<MappingRecord> {
        self.all().into_iter().find(|r| r.distro == distro && r.folder == folder)
    }

    pub fn upsert(&self, rec: MappingRecord, now: u64) -> io::Result<()> {
        let mut data = match self.read() {
            Ok(d) => d,
            Err(_) => {
                let aside = self.path.with_file_name(format!("{MAPPINGS_FILE}.invalid-{now}"));
                fs::rename(&self.path, aside)?;
                FileData::default()
            }
        };
        data.version = 1;
        data.mappings.retain(|r| !(r.distro == rec.distro && r.folder == rec.folder));
        data.mappings.push(rec);
        self.write(&data)
    }

    pub fn remove(&self, distro: &str, folder: &str) -> io::Result<()> {
        let Ok(mut data) = self.read() else { return Ok(()) };
        let before = data.mappings.len();
        data.mappings.retain(|r| !(r.distro == distro && r.folder == folder));
        if data.mappings.len() == before {
            return Ok(());
        }
        self.write(&data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "vlinux-mappings-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn rec(distro: &str, folder: &str, replaced: Replaced) -> MappingRecord {
        MappingRecord {
            distro: distro.into(),
            folder: folder.into(),
            linux_path: format!("/home/me/{folder}"),
            target: "/mnt/d/x".into(),
            windows_path: "D:\\x".into(),
            applied_at: 1,
            replaced,
        }
    }

    #[test]
    fn missing_file_means_no_mappings() {
        let d = temp_dir("none");
        assert!(MappingStore::new(&d).all().is_empty());
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn roundtrip_upsert_find_remove() {
        let d = temp_dir("rt");
        let s = MappingStore::new(&d);
        s.upsert(rec("Debian", "downloads", Replaced::Nothing), 1).unwrap();
        s.upsert(rec("Debian", "documents", Replaced::MovedAside { backup: "/home/me/Documents.vlinux-backup".into() }), 2).unwrap();
        s.upsert(rec("Arch", "downloads", Replaced::EmptyDir), 3).unwrap();
        assert_eq!(s.all().len(), 3);
        assert_eq!(s.for_distro("Debian").len(), 2);
        assert_eq!(s.find("Debian", "documents").unwrap().replaced, Replaced::MovedAside { backup: "/home/me/Documents.vlinux-backup".into() });
        // upsert replaces, it does not duplicate
        s.upsert(rec("Debian", "downloads", Replaced::Link { old_target: "/x".into() }), 4).unwrap();
        assert_eq!(s.for_distro("Debian").len(), 2);
        assert_eq!(s.find("Debian", "downloads").unwrap().replaced, Replaced::Link { old_target: "/x".into() });
        s.remove("Debian", "downloads").unwrap();
        s.remove("Debian", "downloads").unwrap();
        assert!(s.find("Debian", "downloads").is_none());
        assert!(s.find("Arch", "downloads").is_some());
        assert!(!d.join("mappings.json.tmp").exists());
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn corrupt_file_is_kept_and_claims_nothing() {
        let d = temp_dir("corrupt");
        fs::write(d.join(MAPPINGS_FILE), "{ nope").unwrap();
        let s = MappingStore::new(&d);
        assert!(s.all().is_empty());
        s.upsert(rec("Debian", "music", Replaced::Nothing), 42).unwrap();
        assert_eq!(fs::read_to_string(d.join("mappings.json.invalid-42")).unwrap(), "{ nope");
        assert!(s.find("Debian", "music").is_some());
        let _ = fs::remove_dir_all(d);
    }
}
