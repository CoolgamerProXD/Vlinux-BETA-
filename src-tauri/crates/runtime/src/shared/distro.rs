//! The few things that must be done *inside* the distribution rather than through its file share:
//! find the home directory, read and create symbolic links, and the matching clean-up.
//!
//! Every invocation is a fixed `wsl.exe -d <name> --exec <program> <args>` argument list built
//! here (no shell, no string interpolation) and checked against [`is_allowed_distro_op`] before it
//! runs. Paths must be absolute Linux paths without quotes, backslashes or control characters.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use crate::encoding::decode_wsl_output;
use crate::exec::{CmdOutput, CommandRunner, CommandSpec};
use crate::parse::valid_distro_name;

pub const OP_TIMEOUT: Duration = Duration::from_secs(25);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpsError(pub String);

impl fmt::Display for OpsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for OpsError {}

pub trait DistroOps: Send + Sync {
    /// The default user's home directory (absolute Linux path).
    fn home_dir(&self) -> Result<String, OpsError>;
    /// `Some(target)` if `path` is a symbolic link, `None` if it is not (or does not exist).
    fn read_link(&self, path: &str) -> Result<Option<String>, OpsError>;
    /// Create a symbolic link `link` -> `target`. Fails if `link` exists.
    fn make_symlink(&self, target: &str, link: &str) -> Result<(), OpsError>;
    /// Remove a symbolic link (only a link: `unlink` cannot remove a directory).
    fn remove_symlink(&self, link: &str) -> Result<(), OpsError>;
    /// Remove a directory only if it is empty.
    fn remove_empty_dir(&self, path: &str) -> Result<(), OpsError>;
    fn make_dir(&self, path: &str) -> Result<(), OpsError>;
    /// Rename without replacing anything that already exists at `to`.
    fn rename_noclobber(&self, from: &str, to: &str) -> Result<(), OpsError>;
    fn dir_exists(&self, path: &str) -> Result<bool, OpsError>;
}

/// An absolute Linux path that is safe to pass as one argv element.
pub fn valid_exec_path(p: &str) -> bool {
    p.starts_with('/')
        && p.len() <= 4096
        && !p.chars().any(|c| c.is_control() || c == '"' || c == '\\')
        && !p.split('/').any(|c| c == "..")
}

/// The complete set of in-distribution invocations VLINUX may run for storage mapping.
pub fn is_allowed_distro_op(args: &[String]) -> bool {
    let a: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    match a.as_slice() {
        ["-d", name, "--cd", "~", "--exec", "pwd"] => valid_distro_name(name),
        ["-d", name, "--exec", "readlink", "--", p] => valid_distro_name(name) && valid_exec_path(p),
        ["-d", name, "--exec", "ln", "-s", "-T", "--", target, link] | ["-d", name, "--exec", "ln", "-s", "--", target, link] => {
            valid_distro_name(name) && valid_exec_path(target) && valid_exec_path(link)
        }
        ["-d", name, "--exec", "unlink", "--", p] | ["-d", name, "--exec", "rmdir", "--", p] | ["-d", name, "--exec", "mkdir", "--", p] => {
            valid_distro_name(name) && valid_exec_path(p)
        }
        ["-d", name, "--exec", "mv", "-n", "--", from, to] => valid_distro_name(name) && valid_exec_path(from) && valid_exec_path(to),
        ["-d", name, "--exec", "test", "-d", p] => valid_distro_name(name) && valid_exec_path(p),
        _ => false,
    }
}

/// Real implementation: runs the allow-listed commands through `wsl.exe`.
pub struct WslDistroOps {
    runner: Arc<dyn CommandRunner>,
    program: String,
    distro: String,
}

impl WslDistroOps {
    pub fn new(runner: Arc<dyn CommandRunner>, program: impl Into<String>, distro: &str) -> Result<Self, OpsError> {
        if !valid_distro_name(distro) {
            return Err(OpsError(format!("'{distro}' is not a valid distribution name.")));
        }
        Ok(Self { runner, program: program.into(), distro: distro.to_string() })
    }

    fn run(&self, tail: &[&str]) -> Result<CmdOutput, OpsError> {
        let mut args: Vec<&str> = vec!["-d", self.distro.as_str()];
        args.extend_from_slice(tail);
        let spec = CommandSpec::new(self.program.clone(), &args, OP_TIMEOUT).with_env("WSL_UTF8", "1");
        if !is_allowed_distro_op(&spec.args) {
            return Err(OpsError(format!("refused to run a non-allow-listed command: {}", spec.args.join(" "))));
        }
        self.runner.run_simple(&spec).map_err(|e| OpsError(e.to_string()))
    }

    fn text(bytes: &[u8]) -> String {
        decode_wsl_output(bytes).trim_end_matches(['\r', '\n']).to_string()
    }

    fn check(&self, out: &CmdOutput, what: &str) -> Result<(), OpsError> {
        if out.success() {
            return Ok(());
        }
        let err = Self::text(&out.stderr);
        let detail = if err.is_empty() { Self::text(&out.stdout) } else { err };
        Err(OpsError(format!(
            "{what} failed (exit code {}){}",
            out.status.map(|c| c.to_string()).unwrap_or_else(|| "unknown".into()),
            if detail.is_empty() { String::new() } else { format!(": {detail}") }
        )))
    }
}

impl DistroOps for WslDistroOps {
    fn home_dir(&self) -> Result<String, OpsError> {
        let out = self.run(&["--cd", "~", "--exec", "pwd"])?;
        self.check(&out, "Finding the home folder")?;
        let home = Self::text(&out.stdout);
        if valid_exec_path(&home) {
            Ok(home)
        } else {
            Err(OpsError(format!("The distribution reported an unusable home folder: {home:?}")))
        }
    }

    fn read_link(&self, path: &str) -> Result<Option<String>, OpsError> {
        let out = self.run(&["--exec", "readlink", "--", path])?;
        match out.status {
            Some(0) => {
                let t = Self::text(&out.stdout);
                Ok(if t.is_empty() { None } else { Some(t) })
            }
            // readlink exits 1 for "not a symbolic link" and for "does not exist".
            Some(1) => Ok(None),
            _ => self.check(&out, "Reading a link").map(|_| None),
        }
    }

    fn make_symlink(&self, target: &str, link: &str) -> Result<(), OpsError> {
        // `-T`: never create the link *inside* an existing directory. Distributions whose `ln`
        // lacks it (BusyBox) get the plain form; the caller has already checked that `link` is free.
        let out = self.run(&["--exec", "ln", "-s", "-T", "--", target, link])?;
        if out.success() {
            return Ok(());
        }
        let err = Self::text(&out.stderr).to_lowercase();
        if err.contains("invalid option") || err.contains("unrecognized option") || err.contains("usage") {
            let out = self.run(&["--exec", "ln", "-s", "--", target, link])?;
            return self.check(&out, "Creating the link");
        }
        self.check(&out, "Creating the link")
    }

    fn remove_symlink(&self, link: &str) -> Result<(), OpsError> {
        let out = self.run(&["--exec", "unlink", "--", link])?;
        self.check(&out, "Removing the link")
    }

    fn remove_empty_dir(&self, path: &str) -> Result<(), OpsError> {
        let out = self.run(&["--exec", "rmdir", "--", path])?;
        self.check(&out, "Removing the empty folder")
    }

    fn make_dir(&self, path: &str) -> Result<(), OpsError> {
        let out = self.run(&["--exec", "mkdir", "--", path])?;
        self.check(&out, "Creating the folder")
    }

    fn rename_noclobber(&self, from: &str, to: &str) -> Result<(), OpsError> {
        let out = self.run(&["--exec", "mv", "-n", "--", from, to])?;
        self.check(&out, "Moving the folder aside")
    }

    fn dir_exists(&self, path: &str) -> Result<bool, OpsError> {
        let out = self.run(&["--exec", "test", "-d", path])?;
        match out.status {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => self.check(&out, "Checking a folder").map(|_| false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{out_code, out_ok, FakeRunner};

    fn ops(runner: FakeRunner) -> (WslDistroOps, Arc<FakeRunner>) {
        let r = Arc::new(runner);
        (WslDistroOps::new(r.clone(), "wsl.exe", "Debian").unwrap(), r)
    }

    #[test]
    fn allow_list_accepts_only_the_fixed_shapes() {
        let ok = |a: &[&str]| is_allowed_distro_op(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert!(ok(&["-d", "Debian", "--cd", "~", "--exec", "pwd"]));
        assert!(ok(&["-d", "Debian", "--exec", "ln", "-s", "-T", "--", "/mnt/d/x", "/home/me/Downloads"]));
        assert!(ok(&["-d", "Debian", "--exec", "unlink", "--", "/home/me/Downloads"]));
        // Not allowed: other programs, other flags, root, shells, bad names and bad paths.
        assert!(!ok(&["-d", "Debian", "--exec", "rm", "-rf", "/"]));
        assert!(!ok(&["-d", "Debian", "--exec", "sh", "-c", "ls"]));
        assert!(!ok(&["-d", "Debian", "-u", "root", "--exec", "unlink", "--", "/x"]));
        assert!(!ok(&["-d", "Debian", "--exec", "unlink", "/home/me/x"]));
        assert!(!ok(&["-d", "-x", "--exec", "unlink", "--", "/x"]));
        assert!(!ok(&["-d", "Debian", "--exec", "unlink", "--", "relative"]));
        assert!(!ok(&["-d", "Debian", "--exec", "unlink", "--", "/a/../b"]));
        assert!(!ok(&["-d", "Debian", "--exec", "unlink", "--", "/a\"b"]));
        assert!(!ok(&["-d", "Debian", "--exec", "unlink", "--", "/a\nb"]));
        assert!(!ok(&["-d", "Debian", "--exec", "ln", "-s", "-f", "--", "/a", "/b"]));
        assert!(!ok(&["--unregister", "Debian"]));
        assert!(!ok(&["--shutdown"]));
    }

    #[test]
    fn home_dir_uses_the_login_directory() {
        let (o, r) = ops(FakeRunner::new().on("wsl.exe", &["-d", "Debian", "--cd", "~", "--exec", "pwd"], out_ok(b"/home/me\n")));
        assert_eq!(o.home_dir().unwrap(), "/home/me");
        assert_eq!(r.call_lines(), ["wsl.exe -d Debian --cd ~ --exec pwd"]);
    }

    #[test]
    fn a_nonsense_home_is_rejected() {
        let (o, _) = ops(FakeRunner::new().on("wsl.exe", &["-d", "Debian", "--cd", "~", "--exec", "pwd"], out_ok(b"not-a-path\n")));
        assert!(o.home_dir().is_err());
    }

    #[test]
    fn read_link_distinguishes_link_not_a_link_and_failure() {
        let args = ["-d", "Debian", "--exec", "readlink", "--", "/home/me/Downloads"];
        let (o, _) = ops(FakeRunner::new().on("wsl.exe", &args, out_ok(b"/mnt/d/Media/Downloads\n")));
        assert_eq!(o.read_link("/home/me/Downloads").unwrap().as_deref(), Some("/mnt/d/Media/Downloads"));
        let (o, _) = ops(FakeRunner::new().on("wsl.exe", &args, out_code(1, b"")));
        assert_eq!(o.read_link("/home/me/Downloads").unwrap(), None);
        let (o, _) = ops(FakeRunner::new().on("wsl.exe", &args, out_code(127, b"")));
        assert!(o.read_link("/home/me/Downloads").is_err());
    }

    #[test]
    fn make_symlink_falls_back_when_ln_has_no_dash_t() {
        let strict = ["-d", "Debian", "--exec", "ln", "-s", "-T", "--", "/mnt/d/x", "/home/me/D"];
        let plain = ["-d", "Debian", "--exec", "ln", "-s", "--", "/mnt/d/x", "/home/me/D"];
        let busybox = Ok(CmdOutput { status: Some(1), stdout: vec![], stderr: b"ln: invalid option -- 'T'".to_vec() });
        let (o, r) = ops(FakeRunner::new().on("wsl.exe", &strict, busybox).on("wsl.exe", &plain, out_ok(b"")));
        o.make_symlink("/mnt/d/x", "/home/me/D").unwrap();
        assert_eq!(r.calls().len(), 2);
        // A genuine failure (exists) is not retried.
        let exists = Ok(CmdOutput { status: Some(1), stdout: vec![], stderr: b"ln: failed to create symbolic link: File exists".to_vec() });
        let (o, r) = ops(FakeRunner::new().on("wsl.exe", &strict, exists));
        let e = o.make_symlink("/mnt/d/x", "/home/me/D").unwrap_err();
        assert!(e.0.contains("File exists"), "{e}");
        assert_eq!(r.calls().len(), 1);
    }

    #[test]
    fn hostile_paths_never_reach_wsl() {
        let (o, r) = ops(FakeRunner::new());
        assert!(o.remove_symlink("/home/me/x\"; rm -rf /").is_err());
        assert!(o.make_symlink("/ok", "relative").is_err());
        assert!(o.read_link("/a/../b").is_err());
        assert!(r.calls().is_empty());
    }

    #[test]
    fn invalid_distro_names_are_refused() {
        let r: Arc<dyn CommandRunner> = Arc::new(FakeRunner::new());
        assert!(WslDistroOps::new(r.clone(), "wsl.exe", "-d").is_err());
        assert!(WslDistroOps::new(r, "wsl.exe", "a b").is_err());
    }

    #[test]
    fn dir_exists_maps_exit_codes() {
        let args = ["-d", "Debian", "--exec", "test", "-d", "/mnt/d/x"];
        let (o, _) = ops(FakeRunner::new().on("wsl.exe", &args, out_ok(b"")));
        assert!(o.dir_exists("/mnt/d/x").unwrap());
        let (o, _) = ops(FakeRunner::new().on("wsl.exe", &args, out_code(1, b"")));
        assert!(!o.dir_exists("/mnt/d/x").unwrap());
        let (o, _) = ops(FakeRunner::new().on("wsl.exe", &args, out_code(2, b"")));
        assert!(o.dir_exists("/mnt/d/x").is_err());
    }
}
