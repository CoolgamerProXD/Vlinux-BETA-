//! Read-only host information for the status panel. Nothing here changes system state
//! and nothing runs a subprocess.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct SystemInfo {
    pub app_version: String,
    /// "Windows", "Ubuntu" (when developing on Linux), ...
    pub os_name: String,
    /// e.g. "11 (10.0.22631)". Includes the build number on Windows.
    pub os_version: String,
    pub os_arch: String,
    pub cpu_threads: usize,
    pub is_windows: bool,
}

pub fn collect() -> SystemInfo {
    let info = os_info::get();
    SystemInfo {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        os_name: info.os_type().to_string(),
        os_version: info.version().to_string(),
        os_arch: std::env::consts::ARCH.to_string(),
        cpu_threads: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
        is_windows: cfg!(windows),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_plausible_values() {
        let s = collect();
        assert!(!s.app_version.is_empty());
        assert!(!s.os_name.is_empty());
        assert!(s.cpu_threads >= 1);
        assert_eq!(s.is_windows, cfg!(windows));
    }
}
