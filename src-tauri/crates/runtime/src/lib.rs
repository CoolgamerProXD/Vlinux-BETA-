//! VLINUX runtime layer.
//!
//! This crate knows how to *talk to* a Linux runtime (initially WSL2) and nothing about the
//! desktop shell: it has no Tauri dependency, so it can be unit-tested anywhere and a different
//! execution backend can later sit beside it.
//!
//! * [`exec`]      — process execution abstraction (timeouts, cancellation, no shell, mockable)
//! * [`encoding`]  — decoding of `wsl.exe` output (UTF-16LE, BOMs, stray NULs)
//! * [`parse`]     — parsers for `wsl.exe --status/--version/--list` output
//! * [`osrelease`] — `/etc/os-release` parser
//! * [`adapters`]  — distribution adapters + registry (the only place distro specifics live)
//! * [`wsl`]       — allow-listed `wsl.exe` client
//! * [`detect`]    — read-only WSL detection, report and install catalogue
//! * [`install`]   — consent-gated install plans and the install job
//! * [`managed`]   — record of distributions VLINUX installed itself
//! * [`storage`]   — Windows<->Linux shared-folder model, path validation, mapping planner
//! * [`terminal`]  — interactive terminal sessions on a pseudoterminal (M3)
//! * [`testing`]   — fake process runner and helpers for tests

pub mod adapters;
pub mod detect;
pub mod encoding;
pub mod exec;
pub mod install;
pub mod managed;
pub mod model;
pub mod osrelease;
pub mod parse;
pub mod storage;
pub mod terminal;
pub mod testing;
pub mod wsl;

/// Seconds since the Unix epoch (0 if the clock is before it).
pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
