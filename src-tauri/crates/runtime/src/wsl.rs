//! Allow-listed client for `wsl.exe`.
//!
//! VLINUX only ever issues the invocations listed in [`is_allowed_invocation`]. Destructive or
//! configuration-changing verbs (`--unregister`, `--import`, `--export`, `--set-default`,
//! `--set-version`, `--shutdown`, `--mount`, `--manage`, ...) cannot be expressed through this
//! client at all, and are additionally refused by the allow-list as a second line of defence.

use std::sync::Arc;
use std::time::Duration;

use crate::exec::{CancelToken, CmdOutput, CommandRunner, CommandSpec, RunError};
use crate::parse::valid_distro_name;

pub const QUERY_TIMEOUT: Duration = Duration::from_secs(12);
pub const ONLINE_LIST_TIMEOUT: Duration = Duration::from_secs(25);
pub const OS_RELEASE_TIMEOUT: Duration = Duration::from_secs(8);
pub const START_TIMEOUT: Duration = Duration::from_secs(60);
pub const INSTALL_TIMEOUT: Duration = Duration::from_secs(45 * 60);

/// What `wsl.exe --install` should install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallTarget {
    /// One distribution by its official name (from `wsl.exe --list --online`).
    Distribution(String),
    /// WSL itself (Windows features + kernel), without any distribution.
    WslPlatform,
}

/// The program locations used by the runtime layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Programs {
    pub wsl: String,
    pub reg: String,
}

impl Programs {
    /// Absolute `System32` paths when `SystemRoot` is known (avoids PATH hijacking), otherwise bare names.
    pub fn system() -> Self {
        let sys32 = std::env::var_os("SystemRoot").map(|root| std::path::PathBuf::from(root).join("System32"));
        let pick = |exe: &str| -> String {
            if let Some(dir) = &sys32 {
                let p = dir.join(exe);
                if p.exists() {
                    return p.to_string_lossy().into_owned();
                }
            }
            exe.to_string()
        };
        Self { wsl: pick("wsl.exe"), reg: pick("reg.exe") }
    }

    /// Fixed names for tests with a fake runner.
    pub fn fake() -> Self {
        Self { wsl: "wsl.exe".into(), reg: "reg.exe".into() }
    }
}

/// The complete set of `wsl.exe` argument lists VLINUX may run.
pub fn is_allowed_invocation(args: &[String]) -> bool {
    let a: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    match a.as_slice() {
        ["--status"] | ["--version"] => true,
        ["--list", "--verbose"] | ["--list", "--online"] => true,
        ["--terminate", name] => valid_distro_name(name),
        ["--install", "--no-distribution"] => true,
        ["--install", name, "--no-launch"] => valid_distro_name(name),
        ["-d", name, "--exec", "cat", "/etc/os-release"] => valid_distro_name(name),
        ["-d", name, "--exec", "/bin/true"] => valid_distro_name(name),
        _ => false,
    }
}

#[derive(Clone)]
pub struct WslClient {
    runner: Arc<dyn CommandRunner>,
    program: String,
}

impl WslClient {
    pub fn new(runner: Arc<dyn CommandRunner>, program: impl Into<String>) -> Self {
        Self { runner, program: program.into() }
    }

    pub fn runner(&self) -> &Arc<dyn CommandRunner> {
        &self.runner
    }

    fn spec(&self, args: &[&str], timeout: Duration) -> Result<CommandSpec, RunError> {
        // `WSL_UTF8=1` asks newer wsl.exe builds for UTF-8; older ones ignore it and the
        // decoder handles UTF-16.
        let spec = CommandSpec::new(self.program.clone(), args, timeout).with_env("WSL_UTF8", "1");
        if is_allowed_invocation(&spec.args) {
            Ok(spec)
        } else {
            Err(RunError::Io(format!("refused to run a non-allow-listed wsl.exe invocation: {}", args.join(" "))))
        }
    }

    fn run(&self, args: &[&str], timeout: Duration) -> Result<CmdOutput, RunError> {
        let spec = self.spec(args, timeout)?;
        self.runner.run_simple(&spec)
    }

    pub fn status(&self) -> Result<CmdOutput, RunError> {
        self.run(&["--status"], QUERY_TIMEOUT)
    }
    pub fn version(&self) -> Result<CmdOutput, RunError> {
        self.run(&["--version"], QUERY_TIMEOUT)
    }
    pub fn list_verbose(&self) -> Result<CmdOutput, RunError> {
        self.run(&["--list", "--verbose"], QUERY_TIMEOUT)
    }
    /// Needs internet access (the list comes from Microsoft).
    pub fn list_online(&self) -> Result<CmdOutput, RunError> {
        self.run(&["--list", "--online"], ONLINE_LIST_TIMEOUT)
    }

    /// Read-only: `cat /etc/os-release` inside `name` (no shell). On a *stopped* distribution
    /// this starts it, so callers must only do that with the user's say-so.
    pub fn read_os_release(&self, name: &str) -> Result<CmdOutput, RunError> {
        self.run(&["-d", name, "--exec", "cat", "/etc/os-release"], OS_RELEASE_TIMEOUT)
    }

    /// Start a distribution (user-initiated).
    pub fn start(&self, name: &str) -> Result<CmdOutput, RunError> {
        self.run(&["-d", name, "--exec", "/bin/true"], START_TIMEOUT)
    }

    /// Stop one distribution (user-initiated, confirmed in the UI). Does not delete anything.
    pub fn terminate(&self, name: &str) -> Result<CmdOutput, RunError> {
        self.run(&["--terminate", name], QUERY_TIMEOUT)
    }

    /// Run the official installer. Streams output through `on_output`.
    pub fn install(
        &self,
        target: &InstallTarget,
        cancel: &CancelToken,
        on_output: &mut dyn FnMut(&[u8]),
    ) -> Result<CmdOutput, RunError> {
        let spec = match target {
            InstallTarget::Distribution(name) => self.spec(&["--install", name, "--no-launch"], INSTALL_TIMEOUT)?,
            InstallTarget::WslPlatform => self.spec(&["--install", "--no-distribution"], INSTALL_TIMEOUT)?,
        };
        self.runner.run(&spec, cancel, on_output)
    }

    /// The command line shown to the user before they approve an installation.
    pub fn install_command_preview(target: &InstallTarget) -> String {
        match target {
            InstallTarget::Distribution(n) => format!("wsl.exe --install {n} --no-launch"),
            InstallTarget::WslPlatform => "wsl.exe --install --no-distribution".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn allows_exactly_the_documented_invocations() {
        for ok in [
            v(&["--status"]),
            v(&["--version"]),
            v(&["--list", "--verbose"]),
            v(&["--list", "--online"]),
            v(&["--terminate", "Ubuntu"]),
            v(&["--install", "--no-distribution"]),
            v(&["--install", "archlinux", "--no-launch"]),
            v(&["-d", "archlinux", "--exec", "cat", "/etc/os-release"]),
            v(&["-d", "archlinux", "--exec", "/bin/true"]),
        ] {
            assert!(is_allowed_invocation(&ok), "{ok:?}");
        }
    }

    #[test]
    fn refuses_destructive_and_configuration_verbs() {
        for bad in [
            v(&["--unregister", "archlinux"]),
            v(&["-t"]),
            v(&["--import", "x", "C:\\x", "x.tar"]),
            v(&["--export", "x", "x.tar"]),
            v(&["--set-default", "x"]),
            v(&["-s", "x"]),
            v(&["--set-version", "x", "2"]),
            v(&["--shutdown"]),
            v(&["--mount", "\\\\.\\PHYSICALDRIVE0"]),
            v(&["--manage", "x", "--move", "D:\\"]),
            v(&["--update"]),
            v(&["-d", "x", "--exec", "rm", "-rf", "/"]),
            v(&["-d", "x", "--exec", "cat", "/etc/shadow"]),
            v(&["-d", "x", "--", "cat", "/etc/os-release"]),
            v(&["--install", "../evil", "--no-launch"]),
            v(&["--install", "x; calc", "--no-launch"]),
            v(&["--terminate", "-a"]),
            v(&["-d", "--exec", "cat", "/etc/os-release"]),
            v(&[]),
        ] {
            assert!(!is_allowed_invocation(&bad), "{bad:?} must be refused");
        }
    }

    #[test]
    fn client_refuses_unsafe_names_before_reaching_the_runner() {
        use crate::testing::FakeRunner;
        let fake = Arc::new(FakeRunner::new());
        let c = WslClient::new(fake.clone(), "wsl.exe");
        assert!(c.read_os_release("a b").is_err());
        assert!(c.terminate("--all").is_err());
        assert!(c.install(&InstallTarget::Distribution("x;y".into()), &CancelToken::new(), &mut |_| {}).is_err());
        assert!(fake.calls().is_empty(), "nothing may reach the process runner");
    }

    #[test]
    fn previews() {
        assert_eq!(WslClient::install_command_preview(&InstallTarget::Distribution("Debian".into())), "wsl.exe --install Debian --no-launch");
        assert_eq!(WslClient::install_command_preview(&InstallTarget::WslPlatform), "wsl.exe --install --no-distribution");
    }
}
