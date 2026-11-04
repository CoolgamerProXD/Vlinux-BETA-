# VLINUX validation evidence (M6 and M6.1)

This page separates **what was actually run, and where**. Three kinds of evidence exist:

| Label | Meaning |
|---|---|
| **CI-real** | Ran on a GitHub-hosted `windows-latest` runner (Windows Server 2025, build 26100) against the real build/installer/WebView2. Real Windows, **but not the maintainer's PC and with no Linux distribution installed** (WSL reports "no distribution"). |
| **Simulated** | Ran against fake `wsl.exe` output, temp-dir "distribution roots", a fake filesystem or the browser preview. Proves logic, never real-distro behaviour. |
| **Not verified** | Needs the real machine (Windows 11 25H2, existing `archlinux`). Listed so nothing is silently assumed. |

Nothing here was run against the existing `archlinux` distribution. No test unregisters, resets, modifies or starts it; the CI runner has none.

## 1. Easy launch (CI-real)

Source: workflow runs for commits `c41b2a6`, `6dfba9e`, `b0fd483` and `841c8eb` (job *Build, test and smoke-launch on Windows*), step *Installer end-to-end*. Figures below are from `6dfba9e` unless stated.

| Check | Result |
|---|---|
| `npx tauri build` produces an NSIS installer | yes — `VLINUX_0.1.0_x64-setup.exe`, 2.0 MiB (2,082,454 bytes) |
| Silent per-user install (`/S`, no admin) | exit code 0; `vlinux.exe` and `uninstall.exe` installed under `%LOCALAPPDATA%` |
| Start Menu entry | created (the step fails the job if it is missing) |
| Desktop shortcut | created by the silent installer: `Desktop (user) = VLINUX.lnk` and `Start Menu (user) = VLINUX.lnk` (annotation *Installer result*); installed to `%LOCALAPPDATA%\VLINUX` containing `vlinux.exe` and `uninstall.exe` |
| Installed app launches | window shown 44–51 ms after process start (an empty dark window) |
| Desktop appears (not a blank window) | **launch → desktop ready: 858–934 ms** over four runs (process start to `performance.mark('vlinux-ready')`, one clock) |
| Uninstall (silent) | program removed and no shortcuts left (run `c41b2a6`: 0 `vlinux.exe` copies, 0 shortcuts; the step fails the job if `vlinux.exe` remains). Whether the uninstaller leaves `%APPDATA%\app.vlinux.desktop` (your settings) in place is written to `installer-e2e.txt` in the artifact but was **not** read back; it is not claimed here |
| Installer published | GitHub pre-release `installer-<commit>` (job *Publish installer as a pre-release*) |

The installer is **not code-signed** (SmartScreen will warn). Interactive installer pages (a Desktop-shortcut checkbox) were not exercised.

## 2. Real WebView2 (CI-real, 13/13 checks)

`scripts/webview-smoke.mjs` drives a copy of the production build whose window config only adds `--remote-debugging-port` (WebView2 ignored the `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` switch for this host). WebView2/Edge 156.

Passed: desktop renders (shell ready); Tauri IPC bridge present; dock/top bar drawn; 1024×768 window usable; stylesheets applied under the shipped CSP; first-launch welcome shown on a fresh profile; real `distro_overview` IPC answered by Rust; Distributions leaves loading and shows **real** (not simulated) state — "No Linux distribution is installed"; `Alt+F1` opens and `Esc` closes the launcher; all dock apps (except Terminal, which needs a distribution) open; **zero console errors, CSP violations or failed requests** across the run (loads the browser cancelled itself during the script's own reload, `net::ERR_ABORTED`, are not counted — two early runs flagged them and the script was adjusted, nothing else was relaxed).

Not covered: Terminal and Files in WebView2 (need a distribution), full-screen/`F11`, `Ctrl+Alt+Q`, resize by hand, high-DPI.

## 3. Performance — measured on the CI runner (not the Dell)

Installed app, fresh profile, GitHub runner. The runner's CPU, GPU and memory are different from an i5-1335U with Iris Xe; treat these as an order of magnitude only.

| Metric | Value |
|---|---|
| Main process (`vlinux.exe`) working set, idle | 29.6–29.9 MiB (private 4.5 MiB) |
| WebView2 children, idle (6 `msedgewebview2.exe`) | working set sum 308–310 MiB, private sum 113.3 MiB — working sets count shared pages once per process, so the sum over-states real use; private is the better figure |
| Same, after opening Distributions, Setup, System, Settings, Files | main 30.5–30.6 MiB; children working set 420–432 MiB, private 151–157 MiB |
| Idle CPU over 10 s (main + all children) | 0–0.47 % of one core (four runs) |
| Process start → main window shown | 44–51 ms |
| Process start → desktop ready | 858–934 ms |
| Terminal start, filesystem navigation, diagnostics timings | **not measured** — they need a real distribution. Diagnostics records per-check `ms`; run it once on the real PC and read the numbers from the report |

One anomaly, unexplained: in one of four runs the process-tree snapshot after the smoke test listed 131 non-WebView2 descendant processes (the other three runs and the idle snapshots: 0). It was not reproduced when the snapshot was repeated with process names in the next run, so it may be a snapshot artefact, but a burst of child processes from the Setup/System pages cannot be ruled out. Check Task Manager once on the real PC.

Cleanup after exit: the step records how many of the app's child processes are still running 6 s after the main process is killed (in `installer-e2e.txt` in the `vlinux-windows` artifact); that figure was not read back, so **no cleanup claim is made** beyond the real-PTY tests below. Reopening terminals repeatedly is covered only by the Simulated real-PTY tests (`terminal_real_pty`, which open and close real PTYs on `sh`/`cmd.exe` and assert no live session remains).

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

---

# M6.1 — real Arch integration validation and fixes

## 7. What the user confirmed on the real PC (reported, not reproduced here)

| Item | Result |
|---|---|
| Installer runs, desktop launches | **Works** (user-reported) |
| `archlinux` is listed, accessible and can be set as preferred | **Works** (user-reported) |
| Terminal opens; `whoami` | **`SMP`** (user-reported) |
| Diagnostics report, Files browsing, shared folders, desktop shortcuts | **Not yet reported** — see the checklist in section 11 |

## 8. Gaps found, and what was done

| Area | Finding | Action | Evidence |
|---|---|---|---|
| Files / Linux filesystem | Diagnostics only wrote inside `/tmp/vlinux-diag-…`; nothing ever listed or wrote in the real home. | Two new checks: `fs.home_listing` (read-only, counts only, no names) and `fs.home_roundtrip` (needs the write permission): creates ONE hidden uniquely named folder `~/.vlinux-validate-<token>`, then create/write/read/rename/copy/move **inside it**, deletes **only that folder** (path asserted first) and verifies it is gone. Existing files are never opened, renamed, copied or deleted. | 3 new unit tests (existing home files and folders survive; no write without permission; a missing home fails without creating anything). **Simulated** until the PC run. |
| Shared folders | "Apply" could create the link straight from the button. | Apply now always opens a review panel stating the exact change: one new symbolic link, its target, "no file is copied, moved, renamed or deleted", and that **Undo link** removes only that link. | e2e (264/264). |
| Known Folders | Redirected folders are resolved by the OS (`SHGetKnownFolderPath`), correct for OneDrive/other drives, but a OneDrive-synced folder gave no hint that online-only files may not open from Linux. | New verdict **Warning** for any path component starting with `OneDrive`. Linking is still allowed (it is a caveat, not a block). | 1 new unit test. |
| Diagnostics privacy | The Linux user was scrubbed only if the distribution had been inspected first. | The terminal check now adds the user it discovers (`id -un`) to the scrub list for that run's copyable report. | Covered by the existing "no user names" tests; real run still needed. |
| Full-screen toggle | **Real bug, found in the real window:** after leaving full screen, F11 and the top-bar button did nothing (they toggled from a cached flag that the window's change event had not refreshed). | `host.toggleFullscreen()` asks the window for its real state, and the flag is re-read (debounced) on every resize. | 3 new vitest tests + real-window smoke (below). |
| Return to Windows / shortcuts | Never exercised in a real window. | New real-window desktop smoke (section 9). | CI-real. |

## 9. Desktop integration in the REAL window (CI-real, GitHub runner — not the Dell)

`scripts/desktop-smoke.mjs` + `scripts/win-input.ps1` drive the real `vlinux.exe` window under WebView2 on the runner. Keys are **real operating-system key events** (`keybd_event` with hardware scan codes, delivered to the foreground window — a first attempt with `SendKeys` was dropped because it omits scan codes and Chromium then reports an empty `KeyboardEvent.code`; that was a test-tool artefact, not an app bug). A canary types three letters into a text box first; if real keys did not arrive the script says so and falls back to CDP-injected keys, labelling every check.

Last run (commit `9a13473`, **18/18 with real OS keystrokes**):

| Check | Result |
|---|---|
| Starts in full screen (default setting) | pass |
| F11 leaves full screen / F11 enters full screen | pass / pass |
| Real window resized to ~904×601; dock and top bar stay fully inside | pass / pass |
| Alt+F1 opens the launcher, Esc closes it | pass / pass |
| Alt+2 → workspace 2, Alt+1 → workspace 1 | pass / pass |
| Alt+Shift+Q opens the power menu, Esc closes it | pass / pass |
| A window opens from the dock, Alt+W closes it, it opens again | pass / pass / pass |
| **Ctrl+Alt+Q** leaves full screen **and the window is really minimised** (`IsIconic`) | pass |
| Restore from minimised: the desktop is unchanged | pass |
| Top-bar button enters full screen; no script errors | pass / pass |

Honest notes: restoring was done by the test with `ShowWindow(SW_RESTORE)`, not by clicking the taskbar button; the Windows key/Alt+Tab were not exercised; only one monitor of 1024×768; the earlier failing runs (a stale full-screen flag, and my own helper picking WebView2's 16×16 helper window instead of the app window) were fixed before this result.

## 10. Real vs simulated — summary

| Layer | Real? |
|---|---|
| Windows build, installer, shortcuts, launch, uninstall, WebView2 rendering, IPC, CSP, shortcuts, full-screen, resize, minimise | **CI-real** (GitHub runner) |
| WSL detection, listing, state, preferred selection, terminal as `SMP` | **Real on the user's PC** (user-reported) — but no VLINUX test produced evidence for it |
| Terminal I/O, resize, close through WSL | Real PTY on `sh`/`cmd.exe` in CI; **not** through WSL |
| Files in `/home/SMP` over `\\wsl.localhost` | **Not verified** until the home round-trip runs on the PC |
| Shared-folder mapping on the user's Arch | **Not verified, nothing applied**; the engine is tested against temp-dir distro roots |
| `/etc/wsl.conf` read/preview/backup/restore | Simulated |
| Desktop **shortcut/Start Menu entries** | Created and found by the installer e2e on the runner; not opened on the PC |

## 11. Safe tests for the real machine (all optional, all reversible)

**A. Real home-folder round-trip (one click).** Distributions → Diagnostics → tick both permission boxes → Run → Copy report. The `fs.home_listing` and `fs.home_roundtrip` rows say *Real WSL* when they ran against your Arch. The only thing written is one hidden folder `~/.vlinux-validate-<token>` that is removed again; a leftover would be named in the failure message and can be deleted by hand. *Status: unverified until you report it.*

**B. Shared folder, in a dedicated empty folder.**
1. In Windows create an **empty** folder, for example `D:\VlinuxTest` (or any local drive; not a OneDrive folder).
2. VLINUX → Settings → Linux folders → choose an unused folder you do not need (Videos is a good one) → set it to *Shared*, paste the Windows path into the box, press **Check**, then **Use this folder** (this only saves a preference; no link exists yet).
3. **Check status** (looks inside your Arch, and starts it if it is stopped, as the page says): it shows whether the link exists. If `~/Videos` already holds files VLINUX stops and says so; it never replaces a non-empty folder.
4. **Review and apply…**: read the panel (one symbolic link, target, nothing copied/moved/deleted) and confirm. Open Files → `~/Videos`; create a file there and see it in the Windows folder.
5. **Undo link** removes only the link; the Windows folder and its files stay.

Do not point a mapping at Documents, Desktop or Pictures first: they are usually OneDrive-redirected (the app warns), and they hold your real files.


## 12. M6.2 — fixes for the first real-Arch diagnostic

The M6.1 run on the user's Arch (WSL 3.0.3.0) reported 29 passed / 7 failed / 1 not tested. The seven failures fall into three groups. **Root causes below come from reading the code and from the failure pattern; none was observed on a real Windows pseudo-console from this environment.**

| Group | Failure | Cause (best evidence) | Fix |
|---|---|---|---|
| A | `terminal.io`, `identity`, `ansi`, `unicode`, `resize` all timed out at 8 s while the interactive Terminal worked | The diagnostics recorder never answered the pseudo-console's cursor-position request (`ESC[6n`). ConPTY asks it at start-up and holds back output until it is answered. xterm.js answers in the app (and the M3 test collector answers), the diagnostics recorder did not | The recorder answers `ESC[6n` and device-attributes queries (also when split across chunks), acknowledges output, waits up to 20 s for the first answer (typing the harmless command again every 3 s), matches on text with escape sequences removed, uses markers the echoed command cannot contain, polls the resize until `stty size` shows 41 123 and reports what the shell saw otherwise. Failure messages now carry bytes received and queries answered. Later steps are *skipped* (not failed) when the first one did not pass |
| B | `fs.symlink_no_follow` – "Deleting the link reported errors" | Host-side removal of a link over `\\wsl.localhost` failed and the real message was hidden | The file layer classifies links from the entry itself **and** from its parent's directory listing (never descending into a link or any reparse-point folder). If Windows cannot remove a link and it is still there, the link – and only the link – is removed with `unlink` inside the distribution (an existing allow-listed operation), then verified gone. If both fail the message names both reasons. The diagnostic prints the real message |
| C | `fs.cleanup` left `/tmp/vlinux-diag-adac2816d661` | The folder still contained the link from B | Clean-up closes any open diagnostics terminal first, uses the same link-aware delete, retries once, verifies the folder is gone, and otherwise fails naming what remains with an exact `rm -rf` line for that one folder. It never reports a pass while anything remains |

**Not affected:** every check that passed on the PC is unchanged. Nothing is ever unregistered, reset or reinstalled, and `/etc/wsl.conf`, the account and the default distro are not touched.

**Tests added (simulated / real-PTY, none is real WSL):** a ConPTY-like fake shell that prints nothing until its cursor query is answered, with split markers and repaint sequences (all five checks pass, no timeout); a slow shell start; a resize that never arrives (fails naming `24 80`); stripped colour; a silent terminal (one failure, rest skipped, bounded time); recorder query handling incl. split sequences; the exact marker commands on a real Linux PTY (`/bin/sh -i`, real `stty`); link deletion (file, folder and dangling links keep their targets; a locked folder forces the in-distribution fallback; both failing reports both reasons; paths come only from inside the root); a clean-up that cannot finish (fails, names the folder, is retry-safe, then passes) and one that closes a still-open terminal first.

**Retest on the PC:** Distributions → Diagnostics → tick both boxes → Run → Copy report. If any line still fails, its message now says why. If the cleanup line fails, run the `rm -rf` line it prints in the Arch terminal (it only names the diagnostic folder).
