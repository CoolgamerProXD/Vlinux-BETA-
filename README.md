# VLINUX

A Windows application that gives you the experience of a Linux desktop without manual installation: Tauri 2 + Rust core, Svelte/TypeScript shell, WSL2 backend (arriving in M2+).

> **Status: M0 + M1 (foundation + desktop shell).** The shell is a *Windows-rendered* desktop; it is not a Linux compositor and not bare-metal Linux. WSL2 is virtualised. No Linux is started by this version.

## What works now
Full-screen desktop shell, original VLINUX branding and four generated wallpapers, top bar (workspaces, clock/date, network/battery, status, full-screen, power), application launcher with search, dock (bottom or left), floating windows (drag, resize, maximise, minimise, close) across 2–9 workspaces, Settings (appearance, desktop, startup, shortcuts, storage) persisted to TOML, System page, and a power menu with **Return to Windows** (leave full-screen + minimise, nothing stopped) and **Quit** (with confirmation).

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
cd src-tauri && cargo test     # Rust unit tests (config load/save/validation, system info)
```
`.github/workflows/ci.yml` runs all of the above on `windows-latest`, builds the installer and smoke-launches the real app.

## Layout
```
src/            Svelte UI (components/, apps/, lib/)
src-tauri/      Rust core (config, system, window) + Tauri config/capabilities/icons
scripts/        headless-browser e2e test
docs/           ARCHITECTURE.md
```
See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).
