# VLINUX

A Windows application that gives you the experience of a Linux desktop without manual installation: Tauri 2 + Rust core, Svelte/TypeScript shell, WSL2 backend (detection and safe setup in M2).

> **Status: M2 (Linux runtime foundation).** The shell is a *Windows-rendered* desktop; it is not a Linux compositor and not bare-metal Linux. WSL2 is virtualised. VLINUX detects WSL and your distributions read-only, and never starts, installs or changes anything unless you explicitly ask.

## What works now
Full-screen desktop shell, original VLINUX branding and four generated wallpapers, top bar (workspaces, clock/date, network/battery, status, full-screen, power), application launcher with search, dock (bottom or left), floating windows (drag, resize, maximise, minimise, close) across 2–9 workspaces, Settings (appearance, desktop, startup, Linux folders, shortcuts, settings file) persisted to TOML, System page, and a power menu with **Return to Windows** (leave full-screen + minimise, nothing stopped) and **Quit** (with confirmation).

### New in M2 — Linux runtime foundation
* **Linux Setup** app: detects WSL (installed? default version, WSL 2, restart pending, virtualisation problems), lists your distributions (running/stopped, default, WSL version, *Existing* vs *Installed by VLINUX*), and offers to **use your existing distribution** (for example an existing Arch) without touching it. Detection runs only when you open the app or press *Check again*, is cached, and never starts a distribution.
* **Safe install workflow** using only `wsl.exe --install <official name> --no-launch` / `--no-distribution`: official list → review (exact command, admin/restart/internet notes, what stays untouched) → explicit approval checkbox → live progress → cancel. Plans are single-use and expire. Existing distributions are never overwritten, reset, unregistered or modified; failures and leftovers are reported, never auto-removed. **Nothing is installed by the tests or by default.**
* **Distro adapters** (Debian, Ubuntu, Arch, Fedora + generic fallback): the only place distribution specifics live. Adding a distribution means adding an adapter.
* **Shared-folder preferences** (Settings → Linux folders): Downloads/Documents/Pictures/Videos/Music/Desktop → Windows Known Folders (resolved dynamically; redirected folders, other drives, UNC and unsupported paths detected). Saved preferences only — applying them to a distribution arrives with M4.

Terminal (M3), Files (M4) and Distributions (M5) are shown **disabled and labelled** until they exist.

### Shortcuts
`Alt+F1` launcher · `Alt+1…9` workspace · `Alt+W` close window · `F11` full screen · `Alt+Shift+Q` power menu · `Ctrl+Alt+Q` return to Windows immediately.

## Build & run (Windows 10 1809+/11, x64)
Prerequisites: [Rust](https://rustup.rs) (stable, MSVC toolchain + VS Build Tools "Desktop development with C++"), Node.js 20+, WebView2 runtime (preinstalled on Windows 11).
```powershell
npm ci
npm run tauri:dev      # development, hot reload
npm run tauri:build    # release exe + NSIS installer in src-tauri/target/release[/bundle/nsis]
```
The installer is per-user (no admin rights). Uninstalling never touches Linux distributions or your files.

## Tests
```bash
npm run check          # svelte-check / TypeScript
npm test               # vitest unit tests (window manager, config validation, wallpaper generator)
npm run build && npm run e2e   # headless-Chromium end-to-end UI test (SHOTS_DIR=dir for screenshots)
cd src-tauri && cargo test -p vlinux-runtime   # runtime crate (no Tauri needed; fake wsl.exe output; runs anywhere)
cd src-tauri && cargo test --workspace         # + the Tauri app crate (needs the Tauri build prerequisites)
```
`.github/workflows/ci.yml` has a fast Ubuntu job for the runtime crate, and a `windows-latest` job that runs everything, builds the installer and smoke-launches the real app. The browser e2e uses *simulated* WSL data (`?wsl=missing|empty|ready|reboot`, clearly labelled in the UI); real detection only exists in the desktop app.

## Layout
```
src/            Svelte UI (components/, apps/, lib/)
src-tauri/      Rust core (config, system, window, runtime bindings) + Tauri config/capabilities/icons
src-tauri/crates/runtime/   Tauri-free Linux-runtime crate: WSL detection, adapters, install plans, storage model, tests
scripts/        headless-browser e2e test
docs/           ARCHITECTURE.md
```
See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Verification status (M0 + M1)
Verified on a GitHub-hosted `windows-latest` runner (Windows Server 2025, build 26100, Rust 1.99.1): frontend check/tests/build, `cargo test` (7 passed), `tauri build` producing the NSIS installer, and a smoke launch of the real `vlinux.exe` (still running after 15 s, ~30 MiB working set for the main process, 0 % idle CPU for the main process). A screenshot of the real app on Windows is in [docs/screenshots/windows-launch.png](docs/screenshots/windows-launch.png).
Not yet verified by me on a physical Windows 10/11 desktop: installing/uninstalling the NSIS package, multi-monitor behaviour, and the Return-to-Windows minimise/restore flow (the code path is simple but was only exercised in the browser preview, where it correctly reports there is no host window).

## Verification status (M2)
See the M2 section of [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#m2-linux-runtime-foundation) for the design. What is verified, and how:
* **Automated (CI, no real distribution):** `cargo test -p vlinux-runtime` on Ubuntu and `cargo test --workspace --locked` on `windows-latest` use scripted `wsl.exe` output (UTF-16, NULs, missing WSL, no distros, many distros, timeouts, malformed output, pending reboot, install failure/cancel/restart, consent and plan expiry). Frontend: svelte-check, vitest and an 83-check headless-browser e2e against simulated data.
* **Not verified:** behaviour against a real `wsl.exe`, a real WSL install or any real distribution (none exists on CI runners, and VLINUX installs none). No distribution is claimed to be compatible beyond what its adapter identifies from `/etc/os-release` and the registered name; adapters for Debian, Ubuntu, Arch and Fedora have been tested against *fake* output only.
* Shared-folder mappings are **not applied** in M2 (preferences + validation only).
