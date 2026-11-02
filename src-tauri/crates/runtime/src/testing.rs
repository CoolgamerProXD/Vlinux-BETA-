//! Test support: a scripted [`CommandRunner`] that records every call, plus encoders for
//! building realistic `wsl.exe` output. Public (not `#[cfg(test)]`) so integration tests and
//! the Tauri crate can use it; it has no effect on production code paths.

use std::sync::Mutex;

use crate::exec::{CancelToken, CmdOutput, CommandRunner, CommandSpec, RunError};

pub type Handler = Box<
    dyn Fn(&CommandSpec, &CancelToken, &mut dyn FnMut(&[u8])) -> Result<CmdOutput, RunError>
        + Send
        + Sync,
>;

struct Rule {
    program: String,
    args: Vec<String>,
    handler: Handler,
}

#[derive(Default)]
pub struct FakeRunner {
    rules: Mutex<Vec<Rule>>,
    calls: Mutex<Vec<CommandSpec>>,
}

impl FakeRunner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Answer an exact `program args...` invocation with a fixed result.
    pub fn on(self, program: &str, args: &[&str], result: Result<CmdOutput, RunError>) -> Self {
        self.on_fn(program, args, move |_, _, _| result.clone())
    }

    /// Answer with a closure (can stream output, inspect the cancel token, ...).
    pub fn on_fn<F>(self, program: &str, args: &[&str], f: F) -> Self
    where
        F: Fn(&CommandSpec, &CancelToken, &mut dyn FnMut(&[u8])) -> Result<CmdOutput, RunError>
            + Send
            + Sync
            + 'static,
    {
        self.rules.lock().unwrap().push(Rule {
            program: program.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            handler: Box::new(f),
        });
        self
    }

    pub fn calls(&self) -> Vec<CommandSpec> {
        self.calls.lock().unwrap().clone()
    }

    /// Every call as `program arg arg`, in order.
    pub fn call_lines(&self) -> Vec<String> {
        self.calls().iter().map(|c| c.display()).collect()
    }
}

impl CommandRunner for FakeRunner {
    fn run(
        &self,
        spec: &CommandSpec,
        cancel: &CancelToken,
        on_output: &mut dyn FnMut(&[u8]),
    ) -> Result<CmdOutput, RunError> {
        self.calls.lock().unwrap().push(spec.clone());
        let rules = self.rules.lock().unwrap();
        match rules
            .iter()
            .find(|r| r.program == spec.program && r.args == spec.args)
        {
            Some(rule) => (rule.handler)(spec, cancel, on_output),
            // Unscripted commands are test failures, not silent successes.
            None => Err(RunError::Io(format!(
                "unexpected command in test: {}",
                spec.display()
            ))),
        }
    }
}

/// What real `wsl.exe` emits when redirected: UTF-16LE, no BOM, CRLF line endings.
pub fn utf16le(text: &str) -> Vec<u8> {
    text.replace('\n', "\r\n")
        .encode_utf16()
        .flat_map(|u| u.to_le_bytes())
        .collect()
}

pub fn out_ok(stdout: &[u8]) -> Result<CmdOutput, RunError> {
    Ok(CmdOutput {
        status: Some(0),
        stdout: stdout.to_vec(),
        stderr: Vec::new(),
    })
}

pub fn out_code(code: i32, stdout: &[u8]) -> Result<CmdOutput, RunError> {
    Ok(CmdOutput {
        status: Some(code),
        stdout: stdout.to_vec(),
        stderr: Vec::new(),
    })
}

pub fn utf16_ok(text: &str) -> Result<CmdOutput, RunError> {
    out_ok(&utf16le(text))
}

pub fn utf16_code(code: i32, text: &str) -> Result<CmdOutput, RunError> {
    out_code(code, &utf16le(text))
}

/// A ready-made [`WslReport`](crate::model::WslReport) for tests: `(name, state, kind, wsl version)`.
/// The adapter comes from the registered name, exactly as real detection does for distributions
/// that were not inspected.
pub fn fake_report(
    rows: &[(
        &str,
        crate::model::DistroState,
        crate::model::DistroKind,
        u8,
    )],
    default: Option<&str>,
) -> crate::model::WslReport {
    use crate::adapters::DistroRegistry;
    use crate::model::{Distro, HostEnv, IdentitySource, Origin, RuntimeStatus, WslReport};
    let registry = DistroRegistry::with_builtin();
    let distros: Vec<Distro> = rows
        .iter()
        .map(|(name, state, kind, ver)| {
            let adapter = registry.from_registered_name(name);
            Distro {
                name: name.to_string(),
                state: *state,
                wsl_version: *ver,
                is_default: default == Some(*name),
                kind: *kind,
                origin: Origin::Existing,
                adapter: adapter.info(),
                identity_source: IdentitySource::RegisteredName,
                os_release: None,
                display_label: adapter.display_name().to_string(),
            }
        })
        .collect();
    WslReport {
        status: if distros.is_empty() {
            RuntimeStatus::NoDistributions
        } else {
            RuntimeStatus::Ready
        },
        host: HostEnv {
            is_windows: true,
            windows_build: Some(22631),
        },
        wsl_version: Some("2.3.26.0".into()),
        kernel_version: None,
        wslg_version: None,
        default_wsl_version: Some(2),
        wsl2_available: Some(true),
        distros,
        default_distro: default.map(|s| s.to_string()),
        suggested_distro: None,
        pending_reboot: None,
        warnings: Vec::new(),
        detected_at: 1_700_000_000,
    }
}
