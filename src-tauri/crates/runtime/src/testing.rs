//! Test support: a scripted [`CommandRunner`] that records every call, plus encoders for
//! building realistic `wsl.exe` output. Public (not `#[cfg(test)]`) so integration tests and
//! the Tauri crate can use it; it has no effect on production code paths.

use std::sync::Mutex;

use crate::exec::{CancelToken, CmdOutput, CommandRunner, CommandSpec, RunError};

pub type Handler =
    Box<dyn Fn(&CommandSpec, &CancelToken, &mut dyn FnMut(&[u8])) -> Result<CmdOutput, RunError> + Send + Sync>;

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
        F: Fn(&CommandSpec, &CancelToken, &mut dyn FnMut(&[u8])) -> Result<CmdOutput, RunError> + Send + Sync + 'static,
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
        match rules.iter().find(|r| r.program == spec.program && r.args == spec.args) {
            Some(rule) => (rule.handler)(spec, cancel, on_output),
            // Unscripted commands are test failures, not silent successes.
            None => Err(RunError::Io(format!("unexpected command in test: {}", spec.display()))),
        }
    }
}

/// What real `wsl.exe` emits when redirected: UTF-16LE, no BOM, CRLF line endings.
pub fn utf16le(text: &str) -> Vec<u8> {
    text.replace('\n', "\r\n").encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
}

pub fn out_ok(stdout: &[u8]) -> Result<CmdOutput, RunError> {
    Ok(CmdOutput { status: Some(0), stdout: stdout.to_vec(), stderr: Vec::new() })
}

pub fn out_code(code: i32, stdout: &[u8]) -> Result<CmdOutput, RunError> {
    Ok(CmdOutput { status: Some(code), stdout: stdout.to_vec(), stderr: Vec::new() })
}

pub fn utf16_ok(text: &str) -> Result<CmdOutput, RunError> {
    out_ok(&utf16le(text))
}

pub fn utf16_code(code: i32, text: &str) -> Result<CmdOutput, RunError> {
    out_code(code, &utf16le(text))
}
