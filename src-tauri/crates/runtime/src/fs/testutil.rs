//! Test helpers: a self-cleaning temporary directory (no extra dependency).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let p = std::env::temp_dir().join(format!(
            "vlinux-fs-{tag}-{}-{n}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&p).expect("create temp dir");
        // Resolve symlinked temp roots (macOS /var, Windows 8.3 names) so comparisons are stable.
        let p = fs::canonicalize(&p).unwrap_or(p);
        TempDir(p)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn join(&self, rel: &str) -> PathBuf {
        let mut p = self.0.clone();
        for c in rel.split('/').filter(|c| !c.is_empty()) {
            p.push(c);
        }
        p
    }

    /// Create a file (and its parents) with `content`.
    pub fn file(&self, rel: &str, content: &str) -> PathBuf {
        let p = self.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&p, content).unwrap();
        p
    }

    pub fn dir(&self, rel: &str) -> PathBuf {
        let p = self.join(rel);
        fs::create_dir_all(&p).unwrap();
        p
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
