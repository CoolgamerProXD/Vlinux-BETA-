use std::process::Command;

fn main() {
    // Which commit this binary was built from: CI provides GITHUB_SHA; a local build asks git.
    let sha = std::env::var("GITHUB_SHA")
        .ok()
        .filter(|s| s.len() >= 7)
        .map(|s| s[..7].to_string())
        .or_else(|| {
            Command::new("git")
                .args(["rev-parse", "--short=7", "HEAD"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=VLINUX_BUILD_SHA={sha}");
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    tauri_build::build()
}
