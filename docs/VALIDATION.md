# VLINUX validation evidence (M6)

This page separates **what was actually run, and where**. Three kinds of evidence exist:

| Label | Meaning |
|---|---|
| **CI-real** | Ran on a GitHub-hosted `windows-latest` runner (Windows Server 2025, build 26100) against the real build/installer/WebView2. Real Windows, **but not the maintainer's PC and with no Linux distribution installed** (WSL reports "no distribution"). |
| **Simulated** | Ran against fake `wsl.exe` output, temp-dir "distribution roots", a fake filesystem or the browser preview. Proves logic, never real-distro behaviour. |
| **Not verified** | Needs the real machine (Windows 11 25H2, existing `archlinux`). Listed so nothing is silently assumed. |

Nothing here was run against the existing `archlinux` distribution. No test unregisters, resets, modifies or starts it; the CI runner has none.

## 1. Easy launch (CI-real)

Source: workflow run for commit `c41b2a6` (job *Build, test and smoke-launch on Windows*), step *Installer end-to-end*.

| Check | Result |
|---|---|
| `npx tauri build` produces an NSIS installer | yes — `VLINUX_0.1.0_x64-setup.exe`, 2.0 MiB (2,082,454 bytes) |
| Silent per-user install (`/S`, no admin) | exit code 0; `vlinux.exe` and `uninstall.exe` installed under `%LOCALAPPDATA%` |
| Start Menu entry | created (the step fails the job if it is missing) |
| Desktop shortcut | see the `Installer result` annotation of the run (the step emits a `::warning` if the silent installer skips it; none was emitted) |
| Installed app launches | window shown 51 ms after process start (an empty dark window) |
| Desktop appears (not a blank window) | **launch → desktop ready: 858 ms** (process start to `performance.mark('vlinux-ready')`, one clock) |
| Uninstall (silent) | program removed, no shortcuts left; user-data folder not removed by the uninstaller |
| Installer published | GitHub pre-release `installer-<commit>` (job *Publish installer as a pre-release*) |

The installer is **not code-signed** (SmartScreen will warn). Interactive installer pages (a Desktop-shortcut checkbox) were not exercised.

## 2. Real WebView2 (CI-real, 13/13 checks)

`scripts/webview-smoke.mjs` drives a copy of the production build whose window config only adds `--remote-debugging-port` (WebView2 ignored the `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` switch for this host). WebView2/Edge 156.

Passed: desktop renders (shell ready); Tauri IPC bridge present; dock/top bar drawn; 1024×768 window usable; stylesheets applied under the shipped CSP; first-launch welcome shown on a fresh profile; real `distro_overview` IPC answered by Rust; Distributions leaves loading and shows **real** (not simulated) state — "No Linux distribution is installed"; `Alt+F1` opens and `Esc` closes the launcher; all dock apps (except Terminal, which needs a distribution) open; **zero console errors, CSP violations or failed requests** across the run.

Not covered: Terminal and Files in WebView2 (need a distribution), full-screen/`F11`, `Ctrl+Alt+Q`, resize by hand, high-DPI.

## 3. Performance — measured on the CI runner (not the Dell)

Installed app, fresh profile, GitHub runner. The runner's CPU, GPU and memory are different from an i5-1335U with Iris Xe; treat these as an order of magnitude only.

| Metric | Value |
|---|---|
| Main process (`vlinux.exe`) working set, idle | 29.6 MiB (private 4.5 MiB) |
| WebView2 children, idle (6 `msedgewebview2.exe`) | working set sum 309.7 MiB, private sum 113.3 MiB — working sets count shared pages once per process, so the sum over-states real use; private is the better figure |
| Same, after opening Distributions, Setup, System, Settings, Files | main 30.5 MiB; children working set 420.1 MiB, private 150.6 MiB |
| Idle CPU over 10 s (main + all children) | 0 % of one core |
| Process start → main window shown | 51 ms |
| Process start → desktop ready | 858 ms |
| Terminal start, filesystem navigation, diagnostics timings | **not measured** — they need a real distribution. Diagnostics records per-check `ms`; run it once on the real PC and read the numbers from the report |

Cleanup after exit: the step records how many of the app's child processes are still running 6 s after the main process is killed (see `installer-e2e.txt` in the `vlinux-windows` artifact). Reopening terminals repeatedly is covered only by the Simulated real-PTY tests (`terminal_real_pty`, which open and close real PTYs on `sh`/`cmd.exe` and assert no live session remains).

## 4. Automated tests

| Suite | Where | Result |
|---|---|---|
| `svelte-check` | local + CI | 0 errors, 0 warnings |
| `vitest` | local + CI | 68/68 (6 files) |
| Browser e2e (`scripts/e2e.mjs`, headless Chromium, **Simulated** WSL data) | local | 263/263 |
| Runtime crate unit tests | Ubuntu CI | 297 passed |
| Runtime crate unit tests | Windows CI | 258 passed (Unix-only symlink tests are `cfg(unix)`) |
| Tauri app crate tests | Windows CI | 12 passed |
| Integration: `detect_scenarios` 26, `install_scenarios` 20 | Ubuntu + Windows CI | all passed |
| Integration: `terminal_real_pty` | Ubuntu 9, Windows 8 | all passed (**real PTYs** on `sh` / `cmd.exe`, not WSL) |
| Real `wsl.exe` under ConPTY with an unregistered name | Windows CI | returns `WSL_E_DISTRO_NOT_FOUND` as expected (the only real `wsl.exe` exercise) |
| Windows build, NSIS build, installer end-to-end, WebView2 smoke | Windows CI | passed |
| `cargo fmt` | CI `lock` job | applied and committed |
| `cargo clippy` | Windows CI (informational) | warnings remain; none are errors |

New in M6: `terminal.identity` diagnostic with unit tests (the echoed command is never mistaken for output; root and a missing HOME fail), 7 e2e checks for first-launch guidance.

## 5. Real WSL — what is and is not validated

| Area | Status |
|---|---|
| WSL detection, listing, metadata, state, start/stop, read-only inspect, preferred selection | **Simulated** (fake `wsl.exe` output) + one real `wsl.exe` call (no distribution). **Not verified on a real distribution.** |
| Terminal open/IO/resize/close | **Simulated/real PTY on `sh` & `cmd.exe`**. **Not verified through real WSL.** |
| Terminal as `SMP`: `whoami` → SMP, `HOME` → `/home/SMP` | **Not verified.** The diagnostic `terminal.identity` checks "non-root user with an absolute HOME" (it cannot know your expected name and deliberately never prints it). Run it, then type `whoami` yourself. |
| Files: browse `/home/SMP`, create/rename/copy/move/delete a test file, symlinks, error handling | **Simulated** (temp dirs, real unix symlinks on Ubuntu). **Not verified over `\\wsl.localhost`.** Diagnostics → Filesystem does this inside `/tmp/vlinux-diag-…` only. |
| Shared folders: Known Folders, redirected folders, other drives, apply/undo/rollback | **Simulated.** Nothing is ever applied to your Arch automatically; apply/undo are manual and confirmed. |
| `/etc/wsl.conf` read/preview/backup/restore, unknown keys and comments preserved | **Simulated** (round-trip tests). Writing through `\\wsl.localhost` usually fails for a non-root default user; VLINUX never elevates. |
| Diagnostics labelling (Real / Host / Simulated / Not tested) | Tested; the real run on the PC is the missing piece. |

## 6. Known limitations

* No code signing; SmartScreen warning.
* Everything involving a real distribution is unverified until run on the target PC (one short workflow is in the README).
* WebView2 remote debugging could not be enabled through the environment variable, so the CDP smoke uses a copy built with the flag; the installed production binary was only checked for launch, memory and CPU, not driven.
* Performance numbers are from a CI runner.
