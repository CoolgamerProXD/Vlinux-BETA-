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
| `npx tauri build` produces an NSIS installer | yes — `VLINUX_0.1.0_x64-setup.exe` at M6 (named `VLINUX_0.6.3_x64-setup.exe` since M6.3), 2.0 MiB |
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

## 13. M6.3 — root-cause investigation on a REAL Arch (disposable CI VM)

M6.2 was reported to fail the same way as M6.1. Instead of guessing again, a **WSL lab** (`.github/workflows/wsl-lab.yml`, `scripts/wsl-lab/`) installs the official Arch image (`wsl --install archlinux --no-launch`, ~27 s) on a **disposable `windows-latest` runner** and runs the app's own diagnostics code against it. This is a **real WSL 3.0.3.0, kernel 6.18.54.1-1, real Arch, real ConPTY, real `\\wsl.localhost` share — but a CI VM (Windows Server 2025, build 26100), not the Dell**. Nothing in the lab touches the user's `archlinux`.

### Evidence (all from the lab; counts and times only, no output text)

| Question | Result |
|---|---|
| Does M6.1's behaviour (terminal queries unanswered) reproduce the reported failure on a real console? | **Yes, exactly.** `terminal.io` failed after the full wait with *4 bytes received* — the pseudo-console's cursor query `ESC[6n` at 18 ms — *0 bytes answered*, nothing else ever arrived. |
| Does answering the query fix it? | **Yes.** Same distro, queries answered: all five terminal checks + close pass; `terminal.io` in 309 ms (warm) — first output 14 ms, 6 reply bytes written. |
| Is there a slow first start? | Yes: on a cold distro the first `terminal.io` took 6.2 s (output kept arriving for 6 s: 2 238 bytes vs 886 warm). **An 8 s step budget would be close to failing even with the answer; M6.2's 20 s start-up allowance covers it.** Plain `wsl.exe --exec` takes ~120 ms warm, ~2.3 s the very first call. |
| How does Windows see WSL links over the share? | Directory links, file links and dangling links all appear as `ReparsePoint` **without** the Directory attribute; `fsutil reparsepoint query` fails. |
| Can Windows delete them? | **No, not reliably.** `Remove-Item`/`File.Delete`: "Could not find a part of the path" (dir and file links); `Directory.Delete`/`rmdir`: "The directory name is invalid"; `cmd del` and any delete of a *dangling* link **report success and leave the link in place**. |
| Does `unlink` inside the distribution work? | Yes; the share drops the entry at once and the target is untouched. |
| Do M6.2's checks pass on a real Arch? | 34 passed / 1 failed (`terminal.identity`: the lab's first run used root because user creation failed) / 1 skipped / 1 not tested. With a normal default user (`lab`) the terminal checks pass too, and with M6.3 `fs.symlink_no_follow` and `fs.cleanup` pass on the real share. |

### Root causes

| Group | Cause | Status |
|---|---|---|
| A. five terminal timeouts | The recorder in M6.1 never answered ConPTY's `ESC[6n`; ConPTY then sends nothing else. **Reproduced on a real console (above).** | Fixed in M6.2 (answers queries), **confirmed on real Arch**. M6.3 adds an evidence line to each terminal message so any future failure shows *when* output arrived, whether queries were answered/acked, whether the marker was in the raw output or only after removing escapes. |
| B. link delete | Windows delete of a WSL link is unreliable (fails with misleading errors, or silently does nothing). M6.2 tried Windows first and used `unlink` only as a fallback after a *failure* — it would still trust a "success" that changed nothing. | **M6.3:** inside the distribution's share, `unlink` runs **first**; every removal is **verified by looking again** (up to 1 s); Windows is only a last resort and is not believed either. Any reparse point on the share counts as a link (never followed, never descended). Failures are structured: what was tried, the distribution's answer, Windows' answer with its OS error code, whether the link remains, whether the target is still reachable. Target never touched. |
| C. leftover diagnostic folder | Consequence of B. | Same fix; clean-up verified on the real share. |
| D. "I ran M6.2 and it was identical" | The report text the user pasted (every terminal check *"after 8 seconds"*, no skips) is **M6.1's** wording; M6.2 would have skipped four checks and waited 20 s. Every installer was named `VLINUX_0.1.0_x64-setup.exe`, so an old download is easy to run by mistake. **This is an inference; it cannot be proven from here.** | M6.3 bumps the version to **0.6.3** (installer `VLINUX_0.6.3_x64-setup.exe`) and stamps **every diagnostics report and the Diagnostics panel with `<version> (<commit>)`**. |

### Tests (labelled)

* **Real WSL (lab, CI VM):** `real_diagnostics_in_the_wsl_lab`, `real_terminal_experiments_in_the_wsl_lab` (unanswered vs answered).
* **Modelled on lab observations:** unlink-first with Windows never asked; a Windows delete that "succeeds" without removing is not believed; a distribution `unlink` that "succeeds" without removing is not believed; the Windows error code is preserved; the report text contains both answers and "the link is still there: yes"; the target survives each time. A fake console that holds output until answered fails with the evidence line and passes with answers; evidence never contains typed or printed text.
* **Real Windows (CI):** a directory junction and a folder symlink are removed without touching their targets; a file held open without delete-sharing blocks a delete, is reported, and a retry after closing succeeds.

### Still not verified

On **your** PC: that the new installer's diagnostics pass (they should, given the lab, but the lab is a different machine), your login shell/dotfiles, and the Windows Explorer file-association check. Run **Distributions → Diagnostics** with both boxes ticked, and check the first line of the report says `Build: VLINUX 0.6.3 (…)`.

## 14. M7 — desktop experience and product polish

Baseline: `2a9071f` (v0.6.3, run `41993091046`). This milestone changes the shell and adds Linux application discovery; it does not touch the terminal/ConPTY code, the file-operation code or the diagnostics engine (the M6.3 fixes are unchanged, and the same CI checks still run).

### What was run, and where

| Layer | Tests | Where it ran |
|---|---|---|
| Frontend unit (vitest) | 106 (was 71): window manager (snap, switcher, move-to-workspace), notifications, shortcut mapping (incl. the terminal pass-through set), launcher search/recents/id validation, config schema 3 sanitising | Linux sandbox |
| Type check (`svelte-check`) | 0 errors, 0 warnings | Linux sandbox |
| Browser end-to-end | 301 (was 264). New: Linux apps in the launcher (**simulated discovery**), launch + failed launch via notifications, bell/unread, shortcut overlay, window switcher (Alt+\` hold/release/Esc), snapping, move to workspace, desktop/dock/window context menus, theme/blur/dock edge/labels/top-bar toggles, terminal & start-up settings, invalid saved values, distribution chip | Headless Chromium, **simulated WSL data** |
| Rust unit | runtime crate (`apps.rs`: 10 new tests on a fake distribution tree) and the Tauri crate (config schema 3, lenient load) | Ubuntu and Windows CI (see the run in the final report) |
| Windows build, installer, installer e2e, WebView2 smoke, desktop keyboard/window smoke | unchanged scripts, extended with Alt+Shift+F1 / Alt+Shift+N / Alt+Shift+← / Alt+Shift+↑ | Windows CI (real WebView2, real OS keystrokes when the session allows) |
| Real WSL | M6.3 lab workflow, unchanged | disposable CI VM (not your PC) |
| **Your PC** | **not run** | — |

### Performance (measured; relative only)

Method (`scripts/perf.mjs`): headless Chromium on a Linux sandbox with **software rendering** and the browser-preview back end; all six apps open; median of 3 runs; same machine, same session for both builds. These are **not** frame rates or CPU figures for a Windows laptop. Raw output: `docs/perf/m6.3-baseline.json`, `docs/perf/m7.json`.

| Metric | `2a9071f` (M6.3) | M7 | Change |
|---|---|---|---|
| DOM nodes, 6 windows open | 1135 | 1186 | +51 (+4.5 %) — top-bar window list, distro chip, bell |
| JS heap after GC | 4.51 MiB | 4.65 MiB | +0.14 MiB |
| Event listeners | 162 | 167 | +5 |
| Idle main-thread busy | 0.01 % | 0.01 % (blur off: 0.02 %) | none measurable |
| Window-drag frame time, median / p95 | 16.7 / 16.8 ms | 16.7 / 16.7 ms (blur off: 16.7 / 16.7) | none — capped at the 60 Hz vsync, so this **cannot** show the benefit of switching blur off; that needs real GPU hardware and is not measured |
| `index.js` | 260,180 B | 305,196 B | **+45,016 B (+17 %)** — the launcher, notification centre, menus, switcher, shortcut table and settings pages |
| `index.css` | 69,458 B | 83,878 B | +14,420 B (+21 %) |
| whole `dist` | 1,052,402 B | 1,115,159 B | +62,757 B (+6 %) |
| xterm chunk (lazy) | 330,931 B | 330,931 B | unchanged |

Honest reading: the shell is a little bigger and does a little more, and idle cost is unchanged in this environment (and on the CI Windows runner idle CPU has been 0–0.47 % in every measured run since M6; the M7 run is reported with the final report). **There is no measured improvement** in this milestone, only no measured regression; the *Light look* and *Backdrop blur* options are there for GPU-limited machines, but their effect on a real integrated GPU is **unmeasured**. Per-activity cost that *was* reduced by design rather than measured: the clock and animations pause while the window is hidden; discovery runs only when the launcher opens (cached for 60 s, no polling); one global launch listener instead of one per launch.

### What is simulated or not verified

* The launcher's Linux app list in the browser e2e is **fake data**, labelled as such. Real discovery is covered by unit tests on a fake file tree and was **not run against a real distribution**, including your Arch. `wsl.exe --exec <program>` runs the program name as written in the `.desktop` file (no login shell, so no profile scripts); bare names are resolved on the non-login PATH, which was confirmed only for `whoami`/`echo` in M6.3. Programs that live in directories outside that PATH (for example `~/.local/bin`, `/snap/bin`, Flatpak exports) may fail to start; VLINUX would show that failure with the error text, not hide it. Discovery only checks that the program exists in a standard directory.
* WSLg itself: whether a given GUI program shows a window depends on your WSL/WSLg version and GPU drivers. VLINUX reports a failed start and a missing `/mnt/wslg`, but cannot see the window.
* Snap/tile/switcher behaviour is verified in the browser and with OS keystrokes in the Windows smoke; **not** on a different DPI/monitor layout.
* Alt+Shift can be bound by Windows to switching keyboard layouts on machines with several layouts; Alt+Shift shortcuts still reach the app but Windows may also change the layout. This is a Windows setting (*Advanced keyboard settings*).
* Contrast: text colours were chosen for readable contrast on all three themes, but no automated contrast audit or screen-reader pass was run.

### Known limitations

No recursive file search (Files lists, sorts, renames, copies, moves and deletes in the open folder only); no management of WSLg windows (they belong to Windows); discovery needs a running distribution; start-up app limited to Nothing/Distributions/Settings; installer unsigned; single-instance is still not enforced.
