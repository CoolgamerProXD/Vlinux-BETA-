//! A fake distribution for tests: a temporary directory acts as the distribution's root, and
//! "Windows folders" are other temporary directories mounted at Linux paths. Symbolic links are
//! *real* symbolic links on the test machine (Unix only), so the mapping tests exercise the same
//! filesystem behaviour that matters: a file written through `~/Downloads` lands in the Windows
//! folder immediately, without copying.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::distro::{DistroOps, OpsError};
use crate::fs::path::linux_components;

pub struct FakeDistro {
    pub root: PathBuf,
    pub home: String,
    mounts: Vec<(String, PathBuf)>,
    fail: Mutex<HashSet<String>>,
    calls: Mutex<Vec<String>>,
}

fn err(msg: impl Into<String>) -> OpsError {
    OpsError(msg.into())
}

impl FakeDistro {
    pub fn new(root: &Path, home: &str) -> Self {
        Self { root: root.to_path_buf(), home: home.to_string(), mounts: Vec::new(), fail: Mutex::new(HashSet::new()), calls: Mutex::new(Vec::new()) }
    }

    /// Make the host directory `host` appear at the Linux path `linux` (like `/mnt/d/Media`).
    pub fn mount(mut self, linux: &str, host: &Path) -> Self {
        self.mounts.push((linux.trim_end_matches('/').to_string(), host.to_path_buf()));
        self
    }

    /// Make the named operation fail from now on ("make_symlink", "remove_empty_dir", ...).
    pub fn fail_on(&self, op: &str) {
        self.fail.lock().unwrap().insert(op.to_string());
    }

    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn enter(&self, op: &str, detail: &str) -> Result<(), OpsError> {
        self.calls.lock().unwrap().push(format!("{op} {detail}"));
        if self.fail.lock().unwrap().contains(op) {
            return Err(err(format!("{op} failed (injected)")));
        }
        Ok(())
    }

    fn host(&self, linux: &str) -> PathBuf {
        let mut p = self.root.clone();
        for c in linux_components(linux).unwrap_or_default() {
            p.push(c);
        }
        p
    }

    /// Where a Linux path *really* is on the host: under a mount, or under the root.
    fn real(&self, linux: &str) -> PathBuf {
        let comps = linux_components(linux).unwrap_or_default();
        for (m, host) in &self.mounts {
            let mc = linux_components(m).unwrap_or_default();
            if comps.len() >= mc.len() && comps[..mc.len()] == mc[..] {
                let mut p = host.clone();
                for c in &comps[mc.len()..] {
                    p.push(c);
                }
                return p;
            }
        }
        self.host(linux)
    }

    fn linux_text(&self, host: &Path) -> String {
        for (m, h) in &self.mounts {
            if let Ok(rest) = host.strip_prefix(h) {
                let mut s = m.clone();
                for c in rest.components() {
                    s.push('/');
                    s.push_str(&c.as_os_str().to_string_lossy());
                }
                return s;
            }
        }
        if let Ok(rest) = host.strip_prefix(&self.root) {
            let mut s = String::new();
            for c in rest.components() {
                s.push('/');
                s.push_str(&c.as_os_str().to_string_lossy());
            }
            return if s.is_empty() { "/".into() } else { s };
        }
        host.display().to_string()
    }
}

impl DistroOps for FakeDistro {
    fn home_dir(&self) -> Result<String, OpsError> {
        self.enter("home_dir", "")?;
        Ok(self.home.clone())
    }

    fn read_link(&self, path: &str) -> Result<Option<String>, OpsError> {
        self.enter("read_link", path)?;
        let host = self.host(path);
        match fs::symlink_metadata(&host) {
            Ok(m) if m.file_type().is_symlink() => {
                let t = fs::read_link(&host).map_err(|e| err(e.to_string()))?;
                Ok(Some(self.linux_text(&t)))
            }
            Ok(_) => Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(err(e.to_string())),
        }
    }

    #[cfg(unix)]
    fn make_symlink(&self, target: &str, link: &str) -> Result<(), OpsError> {
        self.enter("make_symlink", &format!("{target} {link}"))?;
        let host_link = self.host(link);
        if fs::symlink_metadata(&host_link).is_ok() {
            return Err(err("ln: failed to create symbolic link: File exists"));
        }
        std::os::unix::fs::symlink(self.real(target), &host_link).map_err(|e| err(e.to_string()))
    }

    #[cfg(not(unix))]
    fn make_symlink(&self, target: &str, link: &str) -> Result<(), OpsError> {
        self.enter("make_symlink", &format!("{target} {link}"))?;
        Err(err("symbolic links are only simulated on Unix test machines"))
    }

    fn remove_symlink(&self, link: &str) -> Result<(), OpsError> {
        self.enter("remove_symlink", link)?;
        let host = self.host(link);
        match fs::symlink_metadata(&host) {
            Ok(m) if m.file_type().is_symlink() => fs::remove_file(&host).map_err(|e| err(e.to_string())),
            Ok(_) => Err(err("unlink: not a symbolic link")),
            Err(e) => Err(err(e.to_string())),
        }
    }

    fn remove_empty_dir(&self, path: &str) -> Result<(), OpsError> {
        self.enter("remove_empty_dir", path)?;
        fs::remove_dir(self.host(path)).map_err(|e| err(format!("rmdir: {e}")))
    }

    fn make_dir(&self, path: &str) -> Result<(), OpsError> {
        self.enter("make_dir", path)?;
        fs::create_dir(self.host(path)).map_err(|e| err(format!("mkdir: {e}")))
    }

    fn rename_noclobber(&self, from: &str, to: &str) -> Result<(), OpsError> {
        self.enter("rename_noclobber", &format!("{from} {to}"))?;
        let host_to = self.host(to);
        if fs::symlink_metadata(&host_to).is_ok() {
            return Err(err("mv: target exists"));
        }
        fs::rename(self.host(from), host_to).map_err(|e| err(format!("mv: {e}")))
    }

    fn dir_exists(&self, path: &str) -> Result<bool, OpsError> {
        self.enter("dir_exists", path)?;
        Ok(self.real(path).is_dir())
    }
}
