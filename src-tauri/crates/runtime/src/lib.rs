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
//! * [`fs`]        — file manager backend: locations, listing, file operations (M4)
//! * [`shared`]    — shared-storage mappings: apply/undo with conflict safety (M4)
//! * [`storage`]   — Windows<->Linux shared-folder model, path validation, mapping planner
//! * [`lifecycle`]  — start/stop one distribution, de-duplicated and confirmed (M5)
//! * [`inspect`]    — read-only distribution inspection and its cache (M5)
//! * [`probe`]      — allow-listed read-only in-distribution commands (M5)
//! * [`packages`]   — package-manager capabilities, search parsing, reviewable plans (M5)
//! * [`overview`]   — Distribution Manager cards and the preferred-distribution status (M5)
//! * [`terminal`]  — interactive terminal sessions on a pseudoterminal (M3)
//! * [`testing`]   — fake process runner and helpers for tests

pub mod adapters;
pub mod detect;
pub mod encoding;
pub mod exec;
pub mod fs;
pub mod inspect;
pub mod install;
pub mod lifecycle;
pub mod managed;
pub mod model;
pub mod osrelease;
pub mod overview;
pub mod packages;
pub mod parse;
pub mod probe;
pub mod shared;
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
