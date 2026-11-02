//! Read-only probes run inside a distribution (M5).
//!
//! Every probe is a fixed `wsl.exe -d <name> --exec <program> <args>` argument list built here (no
//! shell) and checked against [`is_allowed_probe`] before it runs. Nothing here writes anything.
//! Running one on a *stopped* distribution starts it, so callers only do that with the user's
//! say-so (see `lifecycle` and `inspect`).

use std::sync::Arc;
use std::time::Duration;

use crate::encoding::decode_wsl_output;
use crate::exec::{CancelToken, CmdOutput, CommandRunner, CommandSpec, RunError};
use crate::packages::{valid_search_query, PROBE_BINARIES};
use crate::parse::valid_distro_name;
use crate::shared::distro::valid_exec_path;

pub const PROBE_TIMEOUT: Duration = Duration::from_secs(12);
pub const SEARCH_TIMEOUT: Duration = Duration::from_secs(60);

/// Tokens used by the diagnostics round trip: `vlx-` plus 8–24 lower-case alphanumerics.
pub fn valid_token(t: &str) -> bool {
    t.strip_prefix("vlx-").is_some_and(|r| {
        (8..=24).contains(&r.len())
            && r.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
    })
}

/// A Windows path in forward-slash form for `wslpath -u`: `C:/Windows`.
pub fn valid_win_probe_path(p: &str) -> bool {
    let b = p.as_bytes();
    b.len() >= 3
        && b.len() <= 260
        && b[0].is_ascii_alphabetic()
        && b[1] == b':'
        && b[2] == b'/'
        && !p
            .chars()
            .any(|c| c.is_control() || matches!(c, '"' | '\\' | '*' | '?' | '<' | '>' | '|'))
        && !p.split('/').any(|c| c == "..")
}

/// The complete set of read-only in-distribution commands VLINUX may run for inspection and
/// diagnostics.
pub fn is_allowed_probe(args: &[String]) -> bool {
    let a: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    match a.as_slice() {
        ["-d", name, "--exec", rest @ ..] if valid_distro_name(name) => match rest {
            ["uname", "-r"] | ["uname", "-m"] | ["id", "-un"] | ["id", "-u"] => true,
            [bin, "--version"] => PROBE_BINARIES.iter().any(|b| b == bin),
            ["apt-cache", "search", "--names-only", q]
            | ["pacman", "-Ss", q]
            | ["dnf", "--quiet", "search", q] => valid_search_query(q),
            ["printf", "%s", t] => valid_token(t),
            ["test", "-e" | "-f" | "-d", p] => valid_exec_path(p),
            ["wslpath", "-u", w] => valid_win_probe_path(w),
            ["cat", "/etc/os-release"] => true,
            _ => false,
        },
        _ => false,
    }
}

pub struct Prober<'a> {
    runner: &'a Arc<dyn CommandRunner>,
    program: &'a str,
    distro: &'a str,
    cancel: &'a CancelToken,
}

/// Outcome of a probe that is allowed to "not be there".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Probed {
    Value(String),
    /// Ran, exited non-zero or printed nothing.
    Absent,
}

impl<'a> Prober<'a> {
    pub fn new(
        runner: &'a Arc<dyn CommandRunner>,
        program: &'a str,
        distro: &'a str,
        cancel: &'a CancelToken,
    ) -> Result<Self, String> {
        if !valid_distro_name(distro) {
            return Err(format!(
                "'{}' is not a valid distribution name.",
                distro.chars().take(40).collect::<String>()
            ));
        }
        Ok(Self {
            runner,
            program,
            distro,
            cancel,
        })
    }

    pub fn exec(&self, tail: &[&str], timeout: Duration) -> Result<CmdOutput, RunError> {
        let mut args: Vec<&str> = vec!["-d", self.distro, "--exec"];
        args.extend_from_slice(tail);
        let spec = CommandSpec::new(self.program, &args, timeout).with_env("WSL_UTF8", "1");
        if !is_allowed_probe(&spec.args) {
            return Err(RunError::Io(format!(
                "refused to run a non-allow-listed command: {}",
                spec.args.join(" ")
            )));
        }
        self.runner.run(&spec, self.cancel, &mut |_| {})
    }

    pub fn text(out: &CmdOutput) -> String {
        decode_wsl_output(&out.stdout)
            .trim_end_matches(['\r', '\n'])
            .trim()
            .to_string()
    }

    fn one_line(&self, tail: &[&str]) -> Result<Probed, RunError> {
        let out = self.exec(tail, PROBE_TIMEOUT)?;
        if !out.success() {
            return Ok(Probed::Absent);
        }
        let t = Self::text(&out);
        let line: String = t
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .chars()
            .take(200)
            .collect();
        Ok(if line.is_empty() {
            Probed::Absent
        } else {
            Probed::Value(line)
        })
    }

    pub fn os_release(&self) -> Result<Probed, RunError> {
        let out = self.exec(&["cat", "/etc/os-release"], PROBE_TIMEOUT)?;
        if !out.success() {
            return Ok(Probed::Absent);
        }
        let t = Self::text(&out);
        Ok(if t.is_empty() {
            Probed::Absent
        } else {
            Probed::Value(t)
        })
    }
    pub fn kernel(&self) -> Result<Probed, RunError> {
        self.one_line(&["uname", "-r"])
    }
    pub fn arch(&self) -> Result<Probed, RunError> {
        self.one_line(&["uname", "-m"])
    }
    pub fn user_name(&self) -> Result<Probed, RunError> {
        self.one_line(&["id", "-un"])
    }
    pub fn user_id(&self) -> Result<Probed, RunError> {
        self.one_line(&["id", "-u"])
    }

    /// `<binary> --version`: `Value(first lines)` when present, `Absent` when missing or failing.
    pub fn manager_version(&self, binary: &str) -> Result<Probed, RunError> {
        // A missing binary makes wsl.exe exit non-zero; a timeout is reported as an error.
        let out = self.exec(&[binary, "--version"], PROBE_TIMEOUT)?;
        if !out.success() {
            return Ok(Probed::Absent);
        }
        let t = Self::text(&out);
        let t: String = t.lines().take(6).collect::<Vec<_>>().join("\n");
        Ok(if t.trim().is_empty() {
            Probed::Absent
        } else {
            Probed::Value(t)
        })
    }

    /// Run an already-built search argv (from `PackageSupport::search_argv`).
    pub fn search(&self, argv: &[String]) -> Result<CmdOutput, RunError> {
        let tail: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
        self.exec(&tail, SEARCH_TIMEOUT)
    }

    /// `printf %s <token>` echoes the token back: proves command execution and argument passing.
    pub fn echo_token(&self, token: &str) -> Result<Probed, RunError> {
        let out = self.exec(&["printf", "%s", token], PROBE_TIMEOUT)?;
        if !out.success() {
            return Ok(Probed::Absent);
        }
        let t = Self::text(&out);
        Ok(if t.is_empty() {
            Probed::Absent
        } else {
            Probed::Value(t)
        })
    }

    /// `test -e|-f|-d <path>`: `Ok(true/false)`; any other exit code is an error.
    pub fn test_path(&self, flag: &str, path: &str) -> Result<bool, RunError> {
        let out = self.exec(&["test", flag, path], PROBE_TIMEOUT)?;
        match out.status {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            other => Err(RunError::Io(format!(
                "test exited with {}",
                other
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "no status".into())
            ))),
        }
    }

    pub fn wslpath(&self, win: &str) -> Result<Probed, RunError> {
        self.one_line(&["wslpath", "-u", win])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{out_code, out_ok, FakeRunner};

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn allow_list_accepts_exactly_the_fixed_shapes() {
        for ok in [
            &["-d", "Debian", "--exec", "uname", "-r"][..],
            &["-d", "Debian", "--exec", "uname", "-m"],
            &["-d", "Debian", "--exec", "id", "-un"],
            &["-d", "Debian", "--exec", "apt-get", "--version"],
            &["-d", "Debian", "--exec", "zypper", "--version"],
            &[
                "-d",
                "Debian",
                "--exec",
                "apt-cache",
                "search",
                "--names-only",
                "git",
            ],
            &["-d", "Arch", "--exec", "pacman", "-Ss", "g++"],
            &["-d", "Fedora", "--exec", "dnf", "--quiet", "search", "git"],
            &["-d", "Debian", "--exec", "printf", "%s", "vlx-abcdef12"],
            &["-d", "Debian", "--exec", "test", "-f", "/tmp/a b"],
            &["-d", "Debian", "--exec", "wslpath", "-u", "C:/Windows"],
            &["-d", "Debian", "--exec", "cat", "/etc/os-release"],
        ] {
            assert!(is_allowed_probe(&args(ok)), "{ok:?}");
        }
    }

    #[test]
    fn allow_list_rejects_everything_else() {
        for bad in [
            &["-d", "Debian", "--exec", "sh", "-c", "id"][..],
            &["-d", "Debian", "--exec", "sudo", "id"],
            &["-d", "Debian", "--exec", "rm", "-rf", "/"],
            &["-d", "Debian", "--exec", "apt-get", "install", "git"],
            &["-d", "Debian", "--exec", "apt-get", "upgrade"],
            &["-d", "Debian", "--exec", "pacman", "-Syu"],
            &["-d", "Debian", "--exec", "pacman", "-Ss", "-h"],
            &[
                "-d",
                "Debian",
                "--exec",
                "apt-cache",
                "search",
                "--names-only",
                "a b",
            ],
            &["-d", "Debian", "--exec", "evil", "--version"],
            &["-d", "Debian", "--exec", "printf", "%s", "hello"],
            &["-d", "Debian", "--exec", "printf", "%s", "vlx-ABCDEFGH"],
            &["-d", "Debian", "--exec", "test", "-f", "/tmp/../etc"],
            &["-d", "Debian", "--exec", "test", "-f", "relative"],
            &["-d", "Debian", "--exec", "test", "-x", "/bin/ls"],
            &["-d", "Debian", "--exec", "wslpath", "-u", "C:\\Windows"],
            &["-d", "Debian", "--exec", "wslpath", "-w", "/tmp"],
            &["-d", "-x", "--exec", "uname", "-r"],
            &["-d", "", "--exec", "uname", "-r"],
            &["--shutdown"],
            &["--unregister", "Debian"],
            &["--set-default", "Debian"],
            &["--set-version", "Debian", "2"],
            &["--import", "D", "C:/x", "C:/y.tar"],
            &["--export", "D", "C:/y.tar"],
            &["--mount", "\\\\.\\PHYSICALDRIVE1"],
        ] {
            assert!(!is_allowed_probe(&args(bad)), "{bad:?}");
        }
    }

    #[test]
    fn prober_refuses_bad_distro_names_up_front() {
        let r: Arc<dyn CommandRunner> = Arc::new(FakeRunner::new());
        let c = CancelToken::new();
        assert!(Prober::new(&r, "wsl.exe", "-bad", &c).is_err());
        assert!(Prober::new(&r, "wsl.exe", "a b; c", &c).is_err());
        assert!(Prober::new(&r, "wsl.exe", "", &c).is_err());
    }

    #[test]
    fn prober_runs_through_the_runner_without_a_shell() {
        let fake = Arc::new(
            FakeRunner::new()
                .on(
                    "wsl.exe",
                    &["-d", "Debian", "--exec", "uname", "-r"],
                    out_ok(b"5.15.153.1-microsoft-standard-WSL2\n"),
                )
                .on(
                    "wsl.exe",
                    &["-d", "Debian", "--exec", "pacman", "--version"],
                    out_code(127, b""),
                ),
        );
        let r: Arc<dyn CommandRunner> = fake.clone();
        let c = CancelToken::new();
        let p = Prober::new(&r, "wsl.exe", "Debian", &c).unwrap();
        assert_eq!(
            p.kernel().unwrap(),
            Probed::Value("5.15.153.1-microsoft-standard-WSL2".into())
        );
        assert_eq!(p.manager_version("pacman").unwrap(), Probed::Absent);
        // A command outside the allow-list never reaches the runner.
        assert!(p.exec(&["sh", "-c", "id"], PROBE_TIMEOUT).is_err());
        assert_eq!(fake.calls().len(), 2);
    }

    #[test]
    fn test_path_distinguishes_missing_from_errors() {
        let fake = Arc::new(
            FakeRunner::new()
                .on(
                    "wsl.exe",
                    &["-d", "D", "--exec", "test", "-e", "/a"],
                    out_ok(b""),
                )
                .on(
                    "wsl.exe",
                    &["-d", "D", "--exec", "test", "-e", "/b"],
                    out_code(1, b""),
                )
                .on(
                    "wsl.exe",
                    &["-d", "D", "--exec", "test", "-e", "/c"],
                    out_code(2, b""),
                ),
        );
        let r: Arc<dyn CommandRunner> = fake;
        let c = CancelToken::new();
        let p = Prober::new(&r, "wsl.exe", "D", &c).unwrap();
        assert_eq!(p.test_path("-e", "/a").unwrap(), true);
        assert_eq!(p.test_path("-e", "/b").unwrap(), false);
        assert!(p.test_path("-e", "/c").is_err());
    }
}
