# VLINUX

A Windows application that gives you the experience of a Linux desktop without manual installation: Tauri 2 + Rust core, Svelte/TypeScript shell, WSL2 backend (detection and safe setup in M2).

> **Status: M6 (real-world validation, installer and release readiness).** The shell is a *Windows-rendered* desktop; it is not a Linux compositor and not bare-metal Linux. WSL2 is virtualised. VLINUX detects WSL and your distributions read-only, and never starts, installs or changes anything unless you explicitly ask.

## Quick start (Windows 11 or 10 1809+, x64)

**Install** — download `VLINUX_0.1.0_x64-setup.exe` from the newest **pre-release** on the repository's *Releases* page (one is published by CI for every pushed commit, tagged `installer-<commit>`; the same file is also in the workflow artifact `vlinux-windows`). Run it: it installs for the current user only (no administrator rights) into `%LOCALAPPDATA%\VLINUX` and adds a **Start Menu** entry (and, in the interactive installer, a Desktop shortcut). The installer is **not code-signed**, so Windows SmartScreen will say "unknown publisher": choose *More info → Run anyway*. Uninstalling removes the program only; it never touches WSL distributions or your files.

**Launch** — Start Menu → *VLINUX* (or the Desktop shortcut). It opens full screen on the desktop. `Ctrl+Alt+Q` goes back to Windows at any time, `F11` toggles full screen.

**From source** — needs Rust (MSVC), Node 20+ and the WebView2 runtime:
```powershell
npm ci
npm run tauri:dev
```

**2-minute check on your own PC** (nothing is installed, started or changed unless you press a button):
1. Launch VLINUX → the *Welcome* card explains the steps. Press **Show my distributions**: your existing WSL distributions are listed (read-only, nothing is started). Press **Set as preferred** on yours.
2. **Distributions → Diagnostics → Run** (tick *Allow creating temporary files* and *Allow starting …* if you want the real terminal, filesystem and shared-folder checks; it writes only inside two folders it creates itself — `/tmp/vlinux-diag-…` and one hidden `~/.vlinux-validate-…` in your Linux home — and removes just those; none of your files are touched). Every result is labelled *Real WSL / Host / Simulated / Not tested*. **Copy report** gives you a redacted text you can paste anywhere.
3. Open **Terminal**: run `whoami` and `echo $HOME` yourself; open **Files**: browse your Linux home.

**Shared-folder test (optional, reversible)** — use a dedicated *empty* Windows folder (for example `D:\VlinuxTest`, not OneDrive) and an Arch folder you don't need (for example Videos): Settings → Linux folders → Videos → Shared → paste the path → *Check* → *Use this folder* → *Check status* → *Review and apply…* (it states the one symbolic link it will create; nothing is copied, moved or deleted) → confirm. *Undo link* removes only the link. Details in [docs/VALIDATION.md](docs/VALIDATION.md#11-safe-tests-for-the-real-machine-all-optional-all-reversible).

## What works now
Full-screen desktop shell, original VLINUX branding and four generated wallpapers, top bar (workspaces, clock/date, network/battery, status, full-screen, power), application launcher with search, dock (bottom or left), floating windows (drag, resize, maximise, minimise, close) across 2–9 workspaces, Settings (appearance, desktop, startup, Linux folders, WSL configuration, shortcuts, settings file) persisted to TOML, System page, and a power menu with **Return to Windows** (leave full-screen + minimise, nothing stopped) and **Quit** (with confirmation).

### New in M2 — Linux runtime foundation
* **Linux Setup** app: detects WSL (installed? default version, WSL 2, restart pending, virtualisation problems), lists your distributions (running/stopped, default, WSL version, *Existing* vs *Installed by VLINUX*), and offers to **use your existing distribution** (for example an existing Arch) without touching it. Detection runs only when you open the app or press *Check again*, is cached, and never starts a distribution.
* **Safe install workflow** using only `wsl.exe --install <official name> --no-launch` / `--no-distribution`: official list → review (exact command, admin/restart/internet notes, what stays untouched) → explicit approval checkbox → live progress → cancel. Plans are single-use and expire. Existing distributions are never overwritten, reset, unregistered or modified; failures and leftovers are reported, never auto-removed. **Nothing is installed by the tests or by default.**
* **Distro adapters** (Debian, Ubuntu, Arch, Fedora + generic fallback): the only place distribution specifics live. Adding a distribution means adding an adapter.
* **Shared-folder preferences** (Settings → Linux folders): Downloads/Documents/Pictures/Videos/Music/Desktop → Windows Known Folders (resolved dynamically; redirected folders, other drives, UNC and unsupported paths detected).

### New in M3 — a real Linux terminal
* **Terminal** app: a genuine interactive shell inside your selected WSL distribution, rendered with xterm.js and driven through a Windows pseudoconsole (ConPTY, via `portable-pty`). Arrow keys, history, Tab completion, Ctrl+C and other control keys, full-screen programs (vim, htop…), ANSI colours and Unicode all come from the real shell and tty — nothing is emulated on the frontend.
* **Your distribution, never a guess:** a session runs `wsl.exe -d <registered name> --cd ~` (your distribution's own configured login shell). If that fails immediately, one fallback to `/bin/sh -l` is tried. The name is validated against the current installed list; a removed/missing distro, missing WSL or no distribution gives an explanation pointing to **Linux Setup**. VLINUX never installs a distro, never changes the WSL default, never runs as root on its own.
* **Tabs and windows:** multiple sessions, per-tab distro and status indicator, `+` for the selected distro and `▾` to open another installed distro, Clear, resize that follows the window, Restart for ended shells, and an **Open terminal** button on each distro in Linux Setup. Closing a window or tab ends only its own sessions (with a confirmation if shells are running) and never shuts down WSL or other distros.
* **Streaming, not polling:** output arrives as events with ack-based flow control, so huge output cannot flood the UI. No Linux process exists until you open a session.

### New in M4 — Files and shared Windows/Linux folders
* **Files** app on the real filesystem of your selected distribution (reached through `\\wsl.localhost\<distro>`; direct filesystem APIs, never shell strings) and on your **Windows shared folders** (resolved dynamically from the Windows Known Folders, or your chosen alternative). Sidebar: Home, Desktop, Documents, Downloads, Pictures, Music, Videos, the Linux *File System*, and the Windows folders. Back/forward/parent, breadcrumbs plus a typed Linux path bar (Ctrl+L; `..`, Windows and UNC paths are refused), hidden-file toggle, sortable columns, selection, context menus, F2 / Delete / Ctrl+C / X / V / A, in-window drag and drop (move; Ctrl = copy).
* **Safe by construction:** nothing is ever overwritten (a name clash keeps both: `name (copy)`), delete needs an explicit confirmation and is permanent (no recycle bin), symbolic links are shown, never followed for copy/move/delete, executable file types are never launched from VLINUX, permission errors and missing paths are explained.
* **Large folders:** a folder is listed page by page (200 per page) behind a virtual list, so only visible rows exist in the UI; large files are never read into the frontend; copy/move/delete run as cancellable background jobs with progress events.
* **Shared folders that really link:** Settings → Linux folders can check each of `~/Downloads`, `~/Documents`, `~/Pictures`, `~/Videos`, `~/Music`, `~/Desktop` in the selected distribution and **apply** it as a symbolic link to the Windows folder (through the distribution's own mount, honouring `/etc/wsl.conf`). Files appear on both sides immediately — no copying, no sync. An existing non-empty folder, a file or a foreign link is **never replaced** without an explicit choice (“move my items aside”, or replace an *empty* folder / a link after confirmation); anything that can't be done safely leaves the filesystem unchanged and says why. **Undo** removes only the link VLINUX made and puts back what was there. A folder is shown as *Connected* only after a live check of the distribution; the badges distinguish *preferred · not checked*, *pending (needs your decision)*, *connected*, *unavailable* and *failed*.

### New in M5 — Distribution Manager, lifecycle and compatibility diagnostics
* **Distributions** app (dock + launcher): one card per WSL distribution — name, display name, version, kernel, default user, architecture, package manager, WSL version, state (*Detected, Running, Stopped, Starting, Installing, Inspecting, Unavailable, Failed*), *Preferred* and *Windows default* badges, and when it was last inspected. **A value VLINUX has not actually read says “Not inspected yet”; nothing is guessed from the distribution's name.** Several distributions can be listed, running and operated on at once. Other applications' distributions (`docker-desktop`, …) are shown but every action on them is disabled.
* **Preferred distribution** is VLINUX's own setting (`[runtime] selected_distro` in `config.toml`) and is **separate from the Windows default**: VLINUX never runs `wsl --set-default`. It is never picked for you, never switched silently if it disappears (a clear warning is shown instead), and changing it does not touch terminals that are already open.
* **Lifecycle, only on your explicit action:** *Start* (visible *Starting* state, verified afterwards, one operation per distribution at a time), *Inspect* (read-only: `/etc/os-release`, kernel, architecture, default user, package managers; cached; a stopped distribution is started only if you allow it, and the result says whether it was), and *Stop* for **one** distribution, behind a confirmation that warns about interrupting running work. There is no global shutdown, and no unregister, reset, delete, import, export, mount, set-default or set-version anywhere: those `wsl.exe` verbs are not in the allow-list (a test asserts it).
* **Package-manager adapters** (APT, pacman, DNF, plus *Unknown*): the manager is **probed inside the distribution, not assumed** from its name. Each reports availability, version and what it can do — *detect, version, search* (read-only, on request), *install* and *update* (**show the exact command only**). VLINUX does not install or upgrade packages itself: that needs root and VLINUX never escalates. It validates the package names, shows the exact `sudo …` command it would suggest, says it has *not* run it, and offers to open a terminal in that distribution where you can run it and type your password yourself. apk and zypper are reported as found-but-unsupported.
* **WSL configuration** (Settings → WSL configuration, and the Distributions → Configuration tab): reads the distribution's `/etc/wsl.conf` (automount, mount root, interoperability, Windows PATH, systemd, default user) and shows the Windows-wide `.wslconfig` (memory/CPU/swap limits) **read-only**. Edits are previewed as a diff, need an explicit confirmation, back the file up first (stored in VLINUX's data, 20 per distribution) and can be restored (a restore backs up the file it replaces, so it can be undone too). Unknown keys, comments and line endings in `wsl.conf` are preserved. The new file is read back to verify it was written, but **VLINUX never claims a setting is active** (it cannot see that), says a restart is needed, and **never restarts or stops anything**. If the distribution does not let the Windows side write the file (normal for a non-root default user), nothing is changed, VLINUX does not elevate, and you are shown the proposed content and how to apply it yourself. `.wslconfig` is never written.
* **Compatibility diagnostics** (Distributions → Diagnostics): Runtime, Terminal (PTY create, I/O, ANSI, Unicode, resize, close), Filesystem (`\\wsl.localhost`, listing, create/rename/copy/move, symlinks, permission errors, clean-up), Shared folders (Known Folders, mount root, mount access, link create/verify/reverse, state evaluation) and Desktop (Explorer, path conversion, `wslpath`, unusual file names; file associations are reported *Not tested*). Every check says what it checked, which distribution, whether it modified anything, the result and what to do. Outcomes: *Passed / Failed / Skipped / Unsupported / Not tested*, and every result carries its **evidence** (*Real WSL*, *Windows host only*, *SIMULATED*, *Did not run*). Checks that write need an explicit permission checkbox and run only inside a temporary `/tmp/vlinux-diag-…` folder, of which only that folder is removed; starting a stopped distribution needs a second checkbox. The report is copyable and redacts user names. **A simulated run is labelled as such and cannot count as a real WSL test.**
* **Settings schema:** `schema_version` stays 2 and old files load unchanged. Saving `config.toml` now keeps settings this version does not know (for example from a newer VLINUX) instead of dropping them; comments in `config.toml` are not preserved (it is rewritten by VLINUX).
* **Performance/security:** nothing starts a distribution or runs a Linux process at start-up; no polling (the Distribution Manager registers its two event listeners only while its window is open); discovery is cached with an explicit Refresh; every command validates the distribution name against the registered list and never takes a path, file text or command line from the frontend.

### Shortcuts
`Alt+F1` launcher · `Alt+1…9` workspace · `Alt+W` close window · `F11` full screen · `Alt+Shift+Q` power menu · `Ctrl+Alt+Q` return to Windows immediately. In the Terminal: `Ctrl+Shift+T` new tab · `Ctrl+Shift+W` close tab · `Ctrl+Shift+C/V` copy/paste (plain `Ctrl+C` is the shell's interrupt). The shell shortcuts above still work while the terminal has focus.

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
npm test               # vitest unit tests (window manager, config validation, wallpaper generator, Distribution Manager view logic)
npm run build && npm run e2e   # headless-Chromium end-to-end UI test (SHOTS_DIR=dir for screenshots)
cd src-tauri && cargo test -p vlinux-runtime   # runtime crate (no Tauri needed; fake wsl.exe output + real PTY tests on sh/cmd.exe; runs anywhere)
cd src-tauri && cargo test --workspace         # + the Tauri app crate (needs the Tauri build prerequisites)
```
`.github/workflows/ci.yml` has a fast Ubuntu job for the runtime crate, and a `windows-latest` job that runs everything, builds the installer and smoke-launches the real app. The browser e2e uses *simulated* WSL data (`?wsl=missing|empty|ready|reboot|multi`, clearly labelled in the UI); real detection only exists in the desktop app.

## Verification status (M6.1)
New since M6, all in [docs/VALIDATION.md](docs/VALIDATION.md) (sections 7–11): the Diagnostics run now also lists your Linux home (read-only) and does a create/write/rename/copy/move round-trip inside one hidden folder it creates and removes itself; Apply for shared folders always shows the exact change first; OneDrive folders get a caveat; the copied report scrubs the Linux user it discovers. A **real-window desktop smoke on the CI runner** (real OS keystrokes: F11, Alt+F1, Alt+1/2, Alt+Shift+Q, Alt+W, Ctrl+Alt+Q with a real minimise, resize) passed 18/18 and found one real bug (F11/full-screen button doing nothing after leaving full screen), now fixed. What you reported (installer, desktop, `archlinux` selectable, Terminal, `whoami` → `SMP`) is recorded as user-reported. **Still unverified on the PC:** Files over `\\wsl.localhost`, the home round-trip, shared-folder apply/undo, and the shortcut icons.

## Verification status (M6)
Evidence, with what ran where (CI-real on a GitHub Windows runner, Simulated, or Not verified), is in [docs/VALIDATION.md](docs/VALIDATION.md). In short: the installer builds, installs silently per-user with Start Menu **and** Desktop shortcuts, launches to the desktop (~0.9 s on the runner), passes a 13-check smoke test inside the real WebView2 with no console/CSP errors, and uninstalls. **Nothing has been verified against a real WSL distribution** — that is the 2-minute check above.

## Verification status (M5)
See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#m5-distribution-management-lifecycle-and-compatibility) for the design.

| Capability | Implemented | Automated tested | Real WSL verified |
|---|---|---|---|
| Discovery of several distributions, states, kinds | Yes | Yes — parsing, fakes, e2e (simulated) | No |
| Preferred distribution (persisted, ≠ Windows default, missing-warning) | Yes | Yes — Rust preferred-status logic (missing/internal/unchecked), validation of the stored name, e2e; the write itself reuses the M2 config save | No |
| Start (explicit, de-duplicated, verified by re-detect) | Yes | Yes — fake `wsl.exe` | No |
| Stop one distribution (confirmation required) | Yes | Yes — fake `wsl.exe`, e2e | No |
| Inspect (read-only, cached, says if it started the distro) | Yes | Yes — fake `wsl.exe` output | No |
| Forbidden `wsl.exe` verbs never issued | Yes (allow-list) | Yes — asserted | n/a |
| Package adapters APT / pacman / DNF / Unknown: probe, version, search | Yes | Yes — fake output | No |
| Package install / update | **Plan only (exact command shown, never executed)** | Yes — plan text, validation, malicious input | n/a |
| `/etc/wsl.conf` read, preview, apply, backup, restore, unknown keys/comments preserved | Yes | Yes — temp files, fault injection | **No** (writing through `\\wsl.localhost` as a non-root user usually fails; that path is handled but unverified) |
| `.wslconfig` (Windows) | Read-only | Yes | No |
| Diagnostics engine (5 categories, outcomes, evidence, redaction, cleanup, cancel) | Yes | Yes — fake `wsl.exe`, temp-dir roots and a scripted terminal (the diagnostics themselves were not run against a real PTY) | **No** — never run against a real distribution |
| Distribution Manager UI, Settings WSL section | Yes | Yes — vitest for view logic; e2e renders it in headless Chromium against a **simulated** backend | No (WebView2 not exercised) |
| `config.toml` unknown-key preservation | Yes (new in M5) | Yes — Windows CI job | n/a |

**What “tested” means here:** the Ubuntu and Windows CI runners have no WSL distribution. Rust tests use fake `wsl.exe` output, temp directories as distribution roots and, for terminals, real PTYs on `sh` / `cmd.exe`. The browser e2e talks to an in-memory **simulation** (labelled *SIMULATED* in the UI and in its diagnostics report). A Windows CI compile or a green simulated run is **not** a real WSL filesystem, terminal or lifecycle test, and no such test has been run. Real-WSL verification of M5 is therefore **not done**; it is the first item for M6.

## Verification status (M4)
See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#m4-files-and-shared-storage) for the design.
* **Temp-dir / fake tests (CI, no distro):** the runtime crate's `fs` and `shared` modules are tested against temp-directory "distribution roots" with a fake `wsl.exe`/distro-ops layer: listing and paging, 2 000+-entry directories, create/rename/copy/move/delete, missing and permission-denied paths, invalid names, traversal (`..`), mount pass-through, redirected Known Folders, UNC and other-drive targets, `/etc/wsl.conf` mount roots, and every mapping state (missing, empty folder, non-empty folder, file, foreign link, already applied, drift, rollback on failure, undo/restore, record store).
* **Real OS tests (CI):** the same code runs against the real filesystem of the Ubuntu runner and, for the portable subset, the Windows runner. Real **symlink** tests run on Ubuntu only (Windows runners can't create symlinks without privileges).
* **UI:** `npm run e2e` drives the real Files and Settings UI against a clearly labelled **simulated** in-memory filesystem (178 checks, including a 3000-entry folder, delete confirmation, drag and drop, conflict flows).
* **Not verified:** anything against a real WSL distribution on a real Windows desktop — symlink behaviour and permissions over `\\wsl.localhost`, `wsl.exe` argument handling for unusual paths, `explorer /select`, opening files with Windows associations, and WebView2 rendering. CI has no distribution.

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
* **CI result (run 41308706302, branch head before this note):** Ubuntu `runtime` job: 60 unit + 26 detection + 20 install tests passed. `windows-latest` job (Server 2025 build 26100): 10 app + 56 runtime unit + 26 + 20 scenario tests passed (`cargo test --workspace --locked`), the Tauri crate compiled, the NSIS installer built, and the smoke launch of `vlinux.exe` stayed up for 15 s (≈30 MiB main-process working set, 0 % idle CPU). Clippy reports 2 minor style warnings.
* **Automated (CI, no real distribution):** `cargo test -p vlinux-runtime` on Ubuntu and `cargo test --workspace --locked` on `windows-latest` use scripted `wsl.exe` output (UTF-16, NULs, missing WSL, no distros, many distros, timeouts, malformed output, pending reboot, install failure/cancel/restart, consent and plan expiry). Frontend: svelte-check, vitest and an 83-check headless-browser e2e against simulated data.
* **Not verified:** behaviour against a real `wsl.exe`, a real WSL install or any real distribution (none exists on CI runners, and VLINUX installs none). No distribution is claimed to be compatible beyond what its adapter identifies from `/etc/os-release` and the registered name; adapters for Debian, Ubuntu, Arch and Fedora have been tested against *fake* output only.
* Shared-folder mappings are **not applied** in M2 (preferences + validation only).

## Verification status (M3)
See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#m3-the-linux-terminal) for the design.
* **Fake-backed tests (no distro):** session open/close/list, invalid and hostile distro names, WSL missing, distro missing/removed, spawn failure and shell fallback, input forwarding, output streaming and flow control, resize, several simultaneous sessions, Unicode, large output, cleanup after failures, and exit reporting. They run on the runtime crate against a scripted backend.
* **Real PTY tests (CI, `sh` on Ubuntu / `cmd.exe` on Windows — not WSL):** a real shell on a real pseudoterminal: echo, Ctrl+C, resize, Unicode, large output, multiple sessions, close/cleanup.
* **Real `wsl.exe` on CI (limited):** on the Windows runner a real `wsl.exe -d <unregistered name>` is run under ConPTY and reports `WSL_E_DISTRO_NOT_FOUND`, proving the Windows ConPTY → `wsl.exe` path and the failure reporting. CI runners have **no** distribution, so an actual Debian/Arch shell has **not** been exercised by me.
* **UI:** svelte-check, vitest and a headless-browser e2e (113 checks) that drives the real xterm.js UI against a labelled loopback transport (it echoes; it is not a shell).
* **Not verified:** an interactive session against a real distribution on a real Windows desktop, WebView2 rendering of xterm.js (the CSP key `dangerousDisableAssetCspModification: ["style-src"]` is untested in a real WebView), clipboard in WebView2, IME input.
