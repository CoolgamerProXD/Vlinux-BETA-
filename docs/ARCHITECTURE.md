# VLINUX architecture (M0 – M5)

## Principles
* **Honest framing.** The VLINUX desktop is a *Windows-rendered shell* (a Tauri/WebView2 window). It is not a Wayland compositor and not a bare-metal Linux install. Linux will run through WSL2, which is virtualised.
* **The shell never needs Linux.** Everything in M0/M1 works with WSL absent; no Linux process is ever started.
* **Windows stays reachable.** Full-screen is ordinary borderless full-screen (never `always_on_top`, never a kiosk hook). Win key, Alt+Tab and the taskbar keep working.
* **Reversible and safe to uninstall.** The installer is per-user NSIS (`installMode: currentUser`, no admin rights). User settings live in `%APPDATA%\app.vlinux.desktop\config.toml`, separate from any distribution's files. Nothing in VLINUX deletes, resets or unregisters distributions.

## Layers
```
Svelte UI (src/)  ──typed invoke──▶  Rust core (src-tauri/src/)  ──▶  Windows / (later) WSL
```
* The UI can only call a fixed set of typed commands (`src/lib/ipc.ts`). There is deliberately **no** generic "run command" bridge. The Tauri capability file grants only `core:event:default`.
* Outside Tauri (plain browser) `ipc.ts` uses browser fallbacks, which is what makes the UI testable headlessly. Those fallbacks never touch a real system and say so in the UI.

### Rust modules
| module | responsibility |
|---|---|
| `config.rs` | Typed, validated TOML config. Enums reject unknown values, numbers are clamped, corrupt files are renamed to `config.toml.invalid-<ts>` (never deleted), writes are atomic. |
| `system.rs` | Read-only host info (OS, version, arch, CPU threads). No subprocesses. |
| `window.rs` | `set_fullscreen`, `is_fullscreen`, `exit_to_windows` (leave full-screen + minimise; nothing terminated), `quit_app` (+ `shutdown_hook` seam for later), full-screen change events. |
| `lib.rs` | Builder, setup (window is created hidden, then configured and shown), command registration. |

| `runtime.rs` | Thin Tauri bindings over the `vlinux-runtime` crate: cached/coalesced detection, catalogue, install plan/start/cancel, distro start/stop, storage overview/validation. No logic of its own beyond state and threading. |

Planned (later milestones): `pty` (ConPTY/`wsl.exe` terminal sessions), `fs`, `distro` (registry + installer).

### Frontend modules
* `lib/wm.svelte.ts` — floating shell-window manager with workspaces (unit-tested).
* `lib/config.svelte.ts` — reactive config, applied live, saved debounced via Rust.
* `lib/host.svelte.ts`, `lib/clock.svelte.ts` — event-driven; **no polling timers**. The clock wakes only on minute (or second) boundaries.
* `lib/apps.ts` — app registry. Apps with `status: 'planned'` render disabled and labelled with their milestone, so there are no buttons that do nothing.
* Wallpapers are generated procedurally from a seed (`wallpaper-gen.ts`): original, tiny, resolution independent.

## Config schema (TOML)
```toml
schema_version = 2   # 2 adds [runtime] and [storage]; version-1 files load unchanged
[shell]      # start_fullscreen, clock_24h, show_seconds, show_date, workspace_count (2-9)
[appearance] # wallpaper = nebula|dusk|peaks|grid, accent = violet|cyan|rose|amber|mint, glass 0-100, animations
[dock]       # position = bottom|left, icon_size 32-72
[runtime]    # selected_distro = "<registered WSL name>"  (absent until the user chooses)
[storage]    # projects = linux_private|shared; per folder [storage.downloads] mode = shared|private, windows_path = "D:\\..." (optional)
```
`src/lib/types.ts` mirrors `config.rs` (including clamping rules).

## Decisions / deviations from the spec
* Stack is as specified (Rust, Tauri 2, Svelte 5 + TS, CSS). xterm.js is deferred to M3.
* Fonts (Inter, JetBrains Mono) are bundled via npm so the UI works offline; CSP is `default-src 'self'`.
* NSIS (per-user) is the only installer target, so no admin prompt is needed to install or uninstall VLINUX.
* Single-instance enforcement is not implemented yet (would add a plugin dependency); listed as a limitation.

## M2: Linux runtime foundation

### Crate split
```
Svelte UI ─typed invoke─▶ src-tauri (Tauri bindings: runtime.rs) ─▶ crates/runtime (vlinux-runtime, no Tauri dependency) ─▶ wsl.exe / reg.exe
```
`vlinux-runtime` knows how to talk to a Linux runtime and nothing about the shell, so it is unit-testable on any OS and a different execution backend can later sit beside the WSL one. Modules:

| module | responsibility |
|---|---|
| `exec` | `CommandRunner` trait (no shell, timeouts, cooperative cancel that kills the child, streaming output), `SystemRunner` (real) |
| `encoding` | decode `wsl.exe` output: UTF-16LE with/without BOM, NUL-interleaved ASCII, UTF-8 (`WSL_UTF8=1`), CRLF |
| `parse` | `--list --verbose`, `--status`, `--version`, `--list --online`, and **locale-independent** failure classification (`aka.ms/wslinstall`, `aka.ms/wslstore`, HRESULTs 0x80370102, 0x8007019e, 0x800701bc, 0x80370114) |
| `osrelease` | `/etc/os-release` parser |
| `adapters` | `DistroAdapter` trait + `DistroRegistry` (Arch, Debian, Ubuntu, Fedora, generic fallback). The only place distro names appear |
| `wsl` | `WslClient`: the **allow-list** of `wsl.exe` invocations |
| `detect` | read-only detection → `WslReport`; install catalogue |
| `install` | install plans, `PlanStore` (consent, expiry, single use), `run_install` |
| `managed` | `distros.json` in the config dir: which distros VLINUX installed |
| `storage` | shared-folder model, Known Folder resolution, path validation, mapping planner |
| `testing` | `FakeRunner` and encoders used by all scenario tests |

### Detection (read-only, lazy, cached)
Flow: host check (Windows and build ≥ 18362) → `wsl --status` → `wsl --version` → `wsl --list --verbose`. Only if WSL is *not* operational are two `reg query` probes run for a pending Windows restart. Result statuses: `unsupported_host`, `not_installed`, `restart_required`, `feature_problem{code,hint}`, `no_distributions`, `ready`, `error{message}`.
* A distribution is **never started to identify it**. Stopped distros are identified by registered name; *running* regular distros get a read-only `cat /etc/os-release` (at most 8 per detection). Distros owned by other tools (`docker-desktop`, `rancher-desktop`, `podman-machine`) are listed as *internal* and never queried, selected or touched.
* The default distro comes from the `*` marker (language independent). Localised state words become `unknown`, not a guess. Unparseable lines produce warnings; unparseable output produces `error`.
* `suggested_distro` = best-ranked existing WSL 2 distro (adapter `existing_install_rank`; Arch is ranked first so an existing Arch is offered) — it is only an offer; nothing is selected automatically and VLINUX never runs `wsl --set-default`.
* Backend caching: 60 s TTL, forced refresh coalesced within 2 s, concurrent callers share one run. The UI detects only when Linux Setup opens or on *Check again*; no timers, nothing at startup.
* `wsl2_available` is `true` only on evidence (kernel version reported or a WSL 2 distro exists); virtualisation can only be proven by actually running a WSL 2 distro, so otherwise it is reported as *not confirmed*.

### Adapter interface
`DistroAdapter`: `id`, `display_name`, `os_release_ids`, `registered_name_prefixes`, `wsl_online_prefixes`, `package_manager` (Apt/Pacman/Dnf/Unknown), `rolling_release`, `notes`, `existing_install_rank`. Resolution order: `ID=`, then each `ID_LIKE` (derivatives such as EndeavourOS → Arch), then the generic adapter. **To support a distribution: implement the trait and `registry.register(...)`** (a test does exactly this). Package install/remove/update, terminal launch and system/filesystem info are deliberately *not* in the trait yet (M3–M5). Only official images from `wsl.exe --list --online` are ever installed; nothing is forked, scraped or repackaged.

### Install workflow and safety rules
`install_prepare` (backend builds a plan from the official catalogue) → UI review → `install_start(plan_id, approved=true)` → events `vlinux://install` → `install_cancel`.
* The frontend never sends a command line. The only commands that can ever be issued are in `wsl::is_allowed_invocation`: `--status`, `--version`, `--list --verbose|--online`, `--terminate <name>`, `--install --no-distribution`, `--install <name> --no-launch`, `-d <name> --exec cat /etc/os-release`, `-d <name> --exec /bin/true`. `--unregister`, `--import`, `--export`, `--set-default`, `--set-version`, `--shutdown`, `--mount`, `--manage` and arbitrary `--exec` are not expressible and are refused again by the allow-list (tested). Names are validated (`[A-Za-z0-9][A-Za-z0-9._-]{0,63}`).
* Consent: unapproved start is refused; plans are single-use, expire after 15 min, and are **re-validated against fresh WSL state** at approval time.
* An existing distribution (case-insensitive) blocks the plan: it is never overwritten, reset or replaced. Distributions without an adapter are listed as *Not supported yet* and cannot be installed. If the official list is unavailable, nothing is installable (it cannot be verified).
* Admin/restart: VLINUX never self-elevates. The WSL platform plan (`--install --no-distribution`) says Windows may show a UAC prompt and may need a restart; restart is detected from exit code 3010, HRESULT 0x80070bc2 or the pending-restart registry key and explained, never performed.
* Failure / cancel / timeout are distinct outcomes. After a failed or cancelled install VLINUX re-detects; a half-registered distro is reported (`leftover_note`) but **never removed**.
* Provenance: when an install succeeded, the name is recorded in `distros.json`; the UI shows *Installed by VLINUX* vs *Existing*. A distro not in that file is always *Existing*.
* One install at a time. `distro_start` / `distro_stop` are explicit user actions (stop asks for confirmation) on regular distros that are in the current WSL report.

### Backend abstraction
The seam for other execution backends is the runtime crate boundary plus `CommandRunner`; WSL2 is the only backend in M2. Nothing in the frontend is distro-specific: it renders `display_label`, adapter package-manager names and badges from the report.

### Storage architecture (preferences; applied in M4 — see the M4 section)
Defaults: `~/Downloads`, `~/Documents`, `~/Pictures`, `~/Videos`, `~/Music`, `~/Desktop` → the matching Windows Known Folder, accessed directly through the drive mount (no copy, no sync). Linux system files, package databases, caches and dotfiles are always private to the distribution. Development projects default to **Linux-native** storage (recommended; `/mnt` access is slower) and can be switched.
* Known Folders are resolved dynamically via the shell folder API (`dirs` → `SHGetKnownFolderPath`), so redirected folders and other drives work; no user name, `C:\Users` path or drive letter is hard-coded.
* `windows_to_wsl(path, mount_root, drives)` is pure and tested. It rejects UNC (`\\server\share`, `\\?\UNC`), `\\wsl$` / `\\wsl.localhost`, device paths, drive-relative paths, `..`, illegal characters, reserved names, trailing space/dot and bare drive roots, and **warns** for network and removable drives (`GetDriveTypeW`). The mount root is assumed to be `/mnt`; reading `/etc/wsl.conf` / `wslpath` is an M4 follow-up.
* `plan_mapping(mode, verdict, existing_linux_entry)` is a pure planner that can never plan data loss: a missing path → create link; the same link → no-op; a symlink elsewhere → retarget (confirmation); an **empty** directory → replace (confirmation); a **non-empty directory or a file → handed to the user, never changed**; unsupported target → blocked; switching to private only unlinks a symlink (confirmation) and leaves the Windows files untouched.
* UI: Settings → *Linux folders* shows, per folder, the Linux path, where it points (or why it cannot be shared), a share/private switch, an alternative Windows folder with a *Check* step, a projects selector and *Restore defaults*. (M4 adds status, apply and undo on top; see below.)

### Testing strategy
* `crates/runtime` unit tests plus `tests/detect_scenarios.rs` and `tests/install_scenarios.rs` drive the real parsing/detection/install code with `FakeRunner` (scripted `wsl.exe`/`reg.exe`). Every detection scenario asserts the read-only guarantee; every install scenario asserts that no destructive verb was issued.
* `src-tauri` tests cover schema-2 config (version-1 files, TOML round-trip, unsafe distro names).
* Frontend: vitest (config validation) and the headless e2e, which uses simulated data (`?wsl=...`, labelled in the UI).
* CI: an Ubuntu job for the crate (fast feedback) and the Windows job for the whole workspace, build, installer and smoke launch.

### Known limitations (M2)
* Behaviour against real `wsl.exe` output on real machines has not been exercised; parsers are built from documented/observed formats and fake fixtures. Localised output is handled where the language-independent markers allow, otherwise reported as unknown/error.
* No terminal, file manager or package operations; selecting a distribution only records the choice.
* A distro that finished installing only after a Windows restart is not recorded as installed by VLINUX (it will show as *Existing*).
* Mount root assumed `/mnt`; custom `automount.root` not read yet.

## M3: the Linux terminal

### Process and PTY architecture
```
xterm.js (Terminal.svelte)  ──typed Tauri commands──▶  src-tauri/src/terminal.rs  ──▶  vlinux-runtime::terminal
   ▲   keystrokes (onData) → terminal_input                 (thin: async + spawn_blocking)     TerminalManager
   │   resize → terminal_resize, ack → terminal_ack                                              │ sessions: id → Session
   └─ events: vlinux://terminal-output {id, data:base64}                                        ▼
            vlinux://terminal-state  {id, state}                      portable-pty (ConPTY on Windows)
                                                                       wsl.exe -d <name> --cd ~   ← argument list built only in Rust
```
* The crate module `terminal/` is Tauri-free: `launch.rs` builds the allowed argument lists, `pty.rs` abstracts the PTY (real `portable-pty` backend, scripted fake for tests), `session.rs` runs one reader thread (output → sink, with ack-based flow control), one waiter thread (exit detection) and the manager, `b64.rs` is a tiny encoder (no extra dependency).
* **Security:** IPC carries only `open{distro|null,rows,cols}`, `input{id,data}`, `resize`, `ack`, `close`, `list`. There is no command-string API. Rust resolves the distro against a fresh/cached detection report (regular, installed, not owned by another app), validates the name, and builds `wsl.exe` arguments from a closed allow-list (`is_allowed_session_args`); no shell or string interpolation is involved. No `--user root`, no elevation, no `--set-default`, no `--shutdown`, `--terminate` or `--unregister`.
* **Launch:** first `wsl.exe -d <name> --cd ~` (the distribution's own configured login shell; nothing hardcoded), and only if it fails immediately one fallback `--exec /bin/sh -l`.
* **Flow control:** output is chunked and base64-encoded; the reader stops after a high-water mark of unacknowledged bytes; xterm acks after writing, so large output cannot flood the WebView.  xterm.js parses writes asynchronously and the ack is sent from its write callback.
* **Lifetime:** nothing runs until `terminal_open`. `close` kills only that session's `wsl.exe` child and closes its PTY. A ConPTY output pipe never reaches EOF on its own, so a waiter closes the master after the child exits and the output drains. `close_all` runs when the app exits; it never touches WSL itself or other distros.
* **Frontend:** `terminal.svelte.ts` store owns xterm instances (lazy-loaded chunk, not part of the startup bundle), `terminal-types.ts` has the pure logic (base64 decode, ack batching, shell-shortcut passthrough), `terminal-preview.ts` is the browser-only loopback (labelled, executes nothing). The window manager gained close **guards** (`wm.guards`, `wm.pendingClose`, `CloseConfirm.svelte`) so closing a Terminal window with live shells asks first.
* **CSP:** xterm.js injects `<style>` elements, so `tauri.conf.json` sets `dangerousDisableAssetCspModification: ["style-src"]` to keep `'unsafe-inline'` effective (Tauri would otherwise add a nonce which disables it). Scripts remain `'self'` only.

### Testing strategy
Fake-PTY tests for every behaviour (see README); real-PTY tests on `sh`/`cmd.exe`; a real `wsl.exe` under ConPTY against an unregistered distro name on the Windows runner (informational); browser e2e against the loopback. A real distro shell is manual-test-only because CI runners have no distribution.

### Known limitations (M3)
* No real-distribution test has been run by me; the Windows desktop WebView2 behaviour of xterm.js is unverified.
* Sessions are not persisted across app restarts; no split panes; no tab reordering; scrollback is 5000 lines.
* Starting a stopped distribution happens implicitly when `wsl.exe -d` runs (that is what WSL does); VLINUX only does it on an explicit user action.
* Interactive programs that rely on Windows-side drag-and-drop, links or images (sixel) are not supported; clipboard uses the WebView clipboard API (Ctrl+Shift+C/V).
* The M2 storage preferences are still not applied (M4).

## M4: Files and shared storage

### Three spaces, never mixed
1. **The Linux distribution filesystem.** Addressed as a Linux path (`/home/me/Downloads`). The backend reaches it with ordinary `std::fs` through the distribution's root as Windows sees it (`\\wsl.localhost\<name>`). It is never reached with shell strings. Only the selected (or explicitly chosen) distribution is used; the distro root is cached per distribution and resolved lazily.
2. **Shared Windows folders.** Addressed as `Windows{root, rel}` where `root` is an id (`downloads`, `documents`, …), never a host path. The backend resolves the id to the real folder: the Windows Known Folder (dynamic, so redirection and other drives work) or the alternative the user chose in Settings. Nothing hard-codes a user name, `C:\Users` or a drive letter.
3. **VLINUX config/app data** (`config.toml`, `distros.json`, `mappings.json` in the app config dir). Never browsable from Files.

The frontend deals only in a typed `Location` (`{space:"linux",path}` or `{space:"windows",root,rel}`). The backend validates every component (no `..`, no NUL, no reserved Windows names, no drive/UNC prefixes) and keeps the result inside its root; a Linux path that crosses a symlink into `/mnt/<drive>` is turned back into the matching shared Windows location or refused (`outside_root`) rather than reaching arbitrary Windows paths.

### Modules
* `crates/runtime/src/fs/` — `error` (typed `FsError`: not_found, permission_denied, already_exists, not_a_directory, is_a_directory, not_empty, invalid_path, invalid_name, outside_root, unavailable, refused, cancelled, io), `path` (validation), `space` (resolving a `Location` against the spaces), `list` (paged, sorted listing sessions; `DEFAULT_PAGE` 200, `MAX_PAGE` 1000), `ops` (create, rename, copy, move, delete; progress, cancellation, report), `filetype` (file classes and the open policy).
* `crates/runtime/src/shared/` — `wslconf` (automount root from `/etc/wsl.conf`), `distro` (the allow-listed `wsl.exe --exec` commands: `pwd`, `readlink`, `ln -s -T`, `unlink`, `rmdir`, `mkdir`, `mv -n`, `test -d`; paths containing quotes, backslashes, control characters or `..` are rejected), `store` (the reversible record in `mappings.json`), `engine` (`status`, `apply`, `undo`), `testing` (fakes).
* `src-tauri/src/files.rs` — 14 async Tauri commands (`fs_places`, `fs_list_open/page/close`, `fs_create`, `fs_rename`, `fs_delete`, `fs_transfer`, `fs_job_cancel`, `fs_open`, `fs_reveal`, `mapping_status/apply/undo`). Blocking work runs off the main thread. Copy/move/delete are background jobs reported by `vlinux://fs-job` events (throttled to 100 ms; at most 6 jobs). Linux processes start only for `fs_places(refresh)`, `mapping_status(check)`, apply and undo.
* Frontend: `fs-types.ts` (shapes and pure helpers), `files.svelte.ts` (store), `mapping.svelte.ts`, `apps/Files.svelte`, the Settings → Linux folders section, and `fs-preview.ts` (the labelled in-memory simulation used only in the browser).

### Files: behaviour
* Listing is a session of sorted entries on the backend; the UI asks for pages as the virtual list scrolls, so a huge directory is never held in the frontend. No polling: lists refresh on navigation, after an operation, or on F5.
* Entries carry `kind` (file, dir, symlink, other), size, modified time, hidden/read-only flags, a file class and an open policy. Symlinks show their target and whether it is a folder; broken links are flagged. Hidden = leading dot.
* **No overwrite, ever:** copy/move into an existing name keeps both (`name (copy)`, `(copy 2)`…); moving a folder into itself is refused; a failed item is reported and the job carries on with the rest. **Delete** requires `confirmed = true` (the UI asks first) and is permanent; symlinks are unlinked, never followed.
* Cross-space copies (Linux ↔ Windows) are streamed in chunks; a move across filesystems is copy-then-delete, and the original is removed only if every item was copied (otherwise it is kept and the problem is reported). Same-filesystem moves use `rename`.
* `fs_open` hands a file to the Windows association only if its class allows it; programs, scripts and shortcuts are refused. `fs_reveal` uses Explorer's `/select`.

### Shared folder mappings: how they are applied and reversed
Mechanism: a **symbolic link inside the distribution**, `~/Downloads → /mnt/<drive>/<path>` (the distribution's own DrvFs mount; the mount root comes from the distribution's `/etc/wsl.conf`, default `/mnt`). Chosen because it needs no Windows feature, no admin rights and no `wsl --mount`, works for any distribution, is visible to Linux tools as a normal path, is trivially reversible, and means both sides see the same bytes. Alternatives considered: bind mounts (need root and `/etc/fstab` edits), `\\wsl.localhost` shares in the other direction (Windows-initiated, not the Linux home), and copying/syncing (explicitly excluded). UNC/network targets are detected and are *not* auto-mounted; they are reported as unavailable unless the distribution already exposes them.

`status` inspects, never changes. States: `unknown` (not yet checked), `private`, `ready` (nothing in the way, or an empty folder that needs confirmation), `applied` (verified by `readlink`), `conflict` (non-empty folder, file, or a link VLINUX didn't create; with the entries listed and the safe offers), `linked_but_private`, `unavailable`, `failed`.

`apply(resolution?)` re-inspects at the moment of acting, so a stale UI can't cause damage, and then: creates the link if nothing exists; with `ReplaceEmptyFolder` removes the *empty* folder (`rmdir` fails on a non-empty one) and links; with `ReplaceLink` swaps a foreign link (the old target is recorded and restored by Undo); with `MoveAside` renames the existing item to `<Name>.vlinux-backup[-N]` (`mv -n`, never overwriting) and links. Every step has a rollback; success is declared only after `readlink` confirms the link. Failure leaves the original in place and says why.

`undo` removes only a link that matches what VLINUX recorded or would create, then restores what was there (the old link, the moved-aside folder, or a fresh empty folder). It never deletes the Windows files or any Linux personal data, and refuses to touch a regular folder or a link it didn't create. Disabling a preference alone changes nothing: the folder shows *Still linked* until the user undoes it.

### Testing strategy
Temp directories serve as distro roots, with a fake distro-ops layer that applies link/rename operations to the temp tree (unix) and a fake `wsl.exe` runner; real `std::fs` behaviour is exercised on the CI OS; real symlinks on Ubuntu. Paths use injected resolvers (Known Folders, drive kinds, mount root) so redirected folders and alternate drives are testable without Windows. The browser e2e uses the in-memory simulation and says so in the UI.

### Known limitations (M4)
* **No real distribution has been tested.** Behaviour of symlinks, permissions and file times over `\\wsl.localhost` (9P), `wsl.exe --exec` quoting for unusual names, and WSL1 differences are unverified; that is why link creation/removal go through `ln`/`unlink` inside the distribution and exec paths exclude quotes and backslashes.
* Links point at `/mnt/<drive>/…`, so they work only while Windows drives are automounted (the status says *unavailable* otherwise). Windows-side tools see the link target through the real folder, not the link.
* Listing a distribution that is stopped starts it (WSL does that on first access); this only happens when the user opens Files/checks status on it.
* Large-folder sorting happens on the backend over the full listing, so the first page of a very large folder takes as long as scanning it; the scan is capped (`truncated` is reported).
* No trash/undo for deletes, no permission editing, no file preview, no search, no multi-window or tabs in Files, no drag and drop from/to Explorer. Open-with uses the default Windows association only.
* UNC/network shared-folder *targets* are browsable from Windows but not auto-linked into Linux.
* Opening files from WSL via `\\wsl.localhost` depends on Windows' handling of that path by the associated program.


## M5: Distribution management, lifecycle and compatibility

### Modules (all Tauri-free in `crates/runtime/src/`, bound by `src-tauri/src/distros.rs`)
* `packages` — the package-manager adapter surface. `PackageSupport::for_manager` (APT, pacman, DNF, *Unknown*) gives the probe binary, a read-only search command and the *plans* for install/update; `capabilities(availability)` returns, per capability (*detect, version, search, install, update*), `available`, `plan_only` or `unsupported` with a reason. Availability is `not_checked | present{version} | absent` and comes from probing, never from the distribution's name. Distro adapters (`adapters.rs`) only say which manager family to expect; distro-specific behaviour stays there and in `packages`.
* `probe` — the one place that runs fixed read-only commands in a distribution (`wsl.exe -d <name> --exec …`, no shell): `cat /etc/os-release`, `uname -r`, `uname -m`, `id -un`, `id -u`, and `<binary> --version` for apt-get/pacman/dnf/apk/zypper (a non-zero exit means *absent*). The allow-list `is_allowed_probe` is checked before any command reaches the runner, so nothing else (no shell, no `sudo`) can be run through it. A stopped distribution is touched only with `allow_start`; the result records `started_by_inspection`.
* `inspect` — builds and caches (`inspections.json`) an `Inspection`. Raw manager probes are stored and capabilities are derived at display time (`resolve_packages`): an unidentified distribution adopts a manager only if exactly one *supported* one is present; apk/zypper are reported found-but-unsupported.
* `lifecycle` — `OpRegistry` (one operation per distribution; a second request is refused as *busy*), `check_target` (name validated, exact match to a registered distribution, internal and mid-change distributions refused), start (`/bin/true` via `--exec`, only if not already running, verified by re-detecting) and stop (`wsl.exe --terminate <name>`, needs `confirmed`, verified). It uses the `WslClient` allow-list, on which the forbidden verbs do not exist; a test asserts `--shutdown --unregister --set-default --set-version --import --export --mount` are all rejected.
* `overview` — `Overview`/`DistroCard`: state (`detected|running|stopped|starting|installing|inspecting|unavailable|failed`), per-action enabled/disabled with a reason, `PreferredStatus` (`none|ok|missing|internal|busy|unchecked`) and the warning text. Internal distributions (docker-desktop etc.) get every action disabled except Refresh.
* `distroconf` — an `IniDoc` that parses `/etc/wsl.conf` keeping every line (comments, unknown keys, ordering, line endings) and edits only six curated settings (`automount.enabled/root`, `interop.enabled/appendWindowsPath`, `boot.systemd`, `user.default`) with typed validation. `plan` builds a diff; `apply` checks the fingerprint, takes a backup in VLINUX's data (`wslconf-backups.json`, 20 per distribution), writes a temp file and renames, reads back, and restores automatically if the read-back differs. `PermissionDenied` becomes `NotWritable` with the proposed text and manual instructions — **no elevation**. `restore` refuses if the file changed since VLINUX wrote it unless forced, and backs up what it replaces. Every outcome has `verified_active: false` and states that a restart is needed.
* `winconf` — a read-only reader for the Windows `.wslconfig` (shown as `%USERPROFILE%\.wslconfig`); never written.
* `diag` — the diagnostics engine (see below).
* `src-tauri/src/distros.rs` — 16 async commands (`distro_overview/set_preferred/start/stop/inspect/inspect_cancel/pkg_plan/pkg_search`, `wslconf_read/preview/apply/backups/restore`, `wslconfig_windows`, `diag_run/cancel`) and the events `vlinux://distro-op`, `vlinux://diag-progress`. Startup only constructs state. Blocking work runs off the main thread. The frontend names a distribution and typed requests; **it never supplies file text** (apply recomputes the plan from the current file and compares `based_on`).

### Discovery and lifecycle
Discovery is the M2 detection (lazy, cached 60 s, read-only, never starts a distribution); `distro_overview(force)` reads it and merges inspections, busy operations and failures. Nothing polls: refreshes happen on window open, on Refresh, and after an operation the user started. Obsolete refreshes are coalesced in the store. Start/Inspect/Stop register in `OpRegistry`, so a double click or a second window cannot run two operations on one distribution. Inspection can be cancelled (`distro_inspect_cancel`); child processes are bounded by timeouts and a cancel token.

### Preferred distribution
`[runtime] selected_distro` (the existing M2 field; no schema change). `distro_set_preferred` validates the name against the registered, non-internal distributions, saves it through the config store and returns a fresh overview. It never calls `wsl --set-default`. If the preferred distribution disappears, the overview warns and **nothing is switched**; terminals already open are not affected because sessions are bound to their own distribution at open time.

### Package capabilities (what is real)
| | APT | pacman | DNF | Unknown |
|---|---|---|---|---|
| Detect / version | probed (`apt-get`, `pacman`, `dnf`) | | | unsupported (adopts a single supported manager if probing finds exactly one) |
| Search | read-only (`apt-cache search --names-only`) | `pacman -Ss` | `dnf --quiet search` | unsupported |
| Install / Update | **plan only** (`sudo apt install …`; `sudo apt update` + `sudo apt upgrade`) | plan only (`sudo pacman -S --needed …`; `-Syu`, with the partial-upgrade warning) | plan only (`sudo dnf install …`; `dnf upgrade --refresh`) | unsupported |

Install/update are never executed by VLINUX: they need root, and VLINUX has no unattended privilege escalation. The plan carries `executed: false`, the exact command text, and a note; the UI offers *Open terminal in this distribution*. Package names are validated (an ASCII letter or digit first, then letters, digits and `+ . _ -`, at most 100 characters; at most 20 names per request), search queries follow the same rule with at most 64 characters, and the command arrays are fixed — no frontend string is ever interpreted by a shell.

### Configuration management
Managed by the distribution: `/etc/wsl.conf` (editable, reversible). Managed by Windows: `.wslconfig` (read-only). Reading requires the distribution to be running (otherwise `needs_start`, and the UI asks you to start it explicitly). Missing, malformed (editing disabled, lines reported), too-large and unsupported cases are all handled before any write. `config.toml` itself: saving now carries over keys this version does not know (`config.rs::carry_unknown`); keys that are skipped when empty (`runtime.selected_distro`, `storage.*.windows_path`) are not resurrected. This is **new in M5** — before it, a newer file's unknown keys were dropped on the next save (invalid files were already kept as `config.toml.invalid-<ts>`). Comments in `config.toml` are not preserved.

### Diagnostics
`diag::run(options, env)` runs the five categories (Runtime, Terminal, Filesystem, Shared folders, Desktop) as a list of named checks. Each `CheckResult` has `outcome` (`passed|failed|skipped|unsupported|not_tested`), `evidence` (`real_wsl|host|simulated|none`), `modified`, `checked`, `message`, `guidance`, `distro`. A check that did not execute has evidence `none` and can never be `passed`. The environment is injected (`Mode::Real` in the desktop app, `Mode::Simulated` only in tests and the browser preview), so a mock cannot produce a *Real WSL* result and the headline of a simulated report says it proves nothing. Writing checks (terminal sessions, create/rename/copy/move, link, odd names) require `allow_write`; starting a stopped distribution requires `allow_start`; both default to off and are shown before the run. All writes go under one `/tmp/vlinux-diag-<token>` folder; cleanup first verifies the exact path and removes only that folder. One run at a time; cancellable. The text report is redacted (user and host names become `<user>`) and holds no file contents. `desktop.file_association` is always *Not tested* — there is no safe read-only query for it yet.

### Frontend
`lib/distros-types.ts` (JSON mirrors), `lib/distros-view.ts` (pure view logic, unit-tested), `lib/distros.svelte.ts` (store; listeners attach on open and detach on close; immediate *Starting/Stopping/Inspecting* states), `apps/Distros.svelte` with `PackagesPanel`, `WslConfigPanel` (also used by Settings → WSL configuration) and `DiagnosticsPanel`, and `lib/distros-preview.ts` — the browser-only **simulation** (`?wsl=multi`: Arch + Windows default running, Ubuntu stopped, Debian on WSL 1, Fedora stopped, Alpine with an unsupported manager, docker-desktop internal). The desktop app never loads it.

### Security restrictions
No `wsl --unregister|--shutdown|--set-default|--set-version|--import|--export|--mount`; no sudo or elevation, no admin request; no package operation at startup or automatic upgrade; no arbitrary command strings over IPC; distribution names validated against the registered list before use; the frontend cannot supply paths or file text; config writes need preview + confirmation + backup; `.wslconfig` is never written; no secrets in diagnostics; symlinks in the diagnostics folder are created and removed only through the allow-listed `ln`/`unlink`.

### Testing strategy
Rust unit tests per module with fake `wsl.exe` output (probe, inspect, lifecycle incl. busy/timeout/cancel, overview, adapters/capabilities, plan validation including hostile strings, `IniDoc` round trips, apply/backup/restore with injected write failures and read-back mismatch, diagnostics classification, cleanup, cancel, redaction). Vitest covers the view logic. The headless-browser e2e renders the Distribution Manager against the simulation (multi-distro states, start/stop/inspect, confirmations, dangerous operations absent, preferred ≠ Windows default, package plans, configuration diff/apply/restore/not-writable, diagnostics, empty and failed discovery, listener clean-up) and re-runs every M0–M4 check.

### Known limitations (M5)
* **Nothing has been run against a real WSL distribution.** Real start/stop/inspect, real `wsl.conf` writes (which will usually be refused for a non-root default user), real diagnostics, `wslpath`, and WebView2 rendering are unverified.
* Package install/update are plan-only by design; there is no in-app package execution or progress.
* Only six `wsl.conf` settings are editable; `.wslconfig` is read-only.
* Diagnostics measure what VLINUX can observe; they do not assert compatibility of any distribution. File associations are not tested.
* Reading `/etc/wsl.conf` goes through `\\wsl.localhost` and needs the distribution running; a distribution that is running but has not finished booting may give transient errors.
* Stopping uses `wsl.exe --terminate <name>` for one distribution; WSL itself decides when the VM shuts down.
* `config.toml` comments are not preserved when VLINUX saves it.

## M6: validation, installer and release readiness

M6 adds no feature; it checks that what exists survives contact with a real Windows build, and makes it easy to start.

* **First launch.** `Welcome.svelte` is shown on the empty desktop while no preferred distribution is chosen and it has not been dismissed (`localStorage` key `vlinux.welcome.dismissed`, UI state only — not in `config.toml`). Its three steps lead to Distributions (read-only detection), Terminal/Files and Diagnostics. Start-up can no longer leave an invisible shell: `ready` is set even if loading the config or host info throws (a toast says what failed), and `index.html` carries a `<noscript>` note.
* **Diagnostics.** New check `terminal.identity` types `printf 'VLXID:%s:%s:END\n' "$(id -un)" "$HOME"` into the diagnostics PTY and reads the answer (only a line without `%`/`$` counts, so the echoed command is never mistaken for output). It passes for a non-root user with an absolute `HOME`; the user name is neither shown nor put in the report.
* **CI (Windows job).** After building, the job installs the real NSIS installer silently (per-user), inventories the install folder and the Start Menu / Desktop shortcuts, launches the *installed* app with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222` and a fresh `WEBVIEW2_USER_DATA_FOLDER`, and drives it with `scripts/webview-smoke.mjs` (puppeteer-core over CDP, no new dependency): desktop rendered, no console/CSP errors, real Tauri IPC (`distro_overview`), Distributions showing real (not simulated) state, launcher shortcuts, dock apps. It records the process tree's memory with the main process and the `msedgewebview2.exe` children **separately**, idle CPU, window-shown and launch-to-desktop times (`performance.mark('vlinux-ready')`), then exits the app, checks for leftover child processes, uninstalls silently and verifies the program is gone. These numbers come from a GitHub runner, not a user's PC.
* **Release.** A `release` job publishes the installer of each pushed (non-`main`) commit as a GitHub pre-release `installer-<sha>`, with its SHA-256.
* **Formatting.** The `lock` job runs `cargo fmt` and commits the result back on branch pushes (a commit pushed with `GITHUB_TOKEN` does not start a new run, so the next push is the verification).

## M6.1 — real-integration validation

* **Diagnostics `fs.home_listing` / `fs.home_roundtrip`** (`runtime/src/diag.rs`): the only diagnostic that touches the user's home. It creates one hidden folder `~/.vlinux-validate-<token>`, performs create/write/read/rename/copy/move strictly inside it, then runs `ops::delete` on exactly that path (asserted against the folder it created; any other path is refused) and verifies removal. It needs the same explicit write permission as the other writing checks, and never opens existing files. Listing reports counts only.
* **Report privacy:** the terminal check adds the user name it discovers (`id -un`) to the per-run scrub list; `/home/<name>` and `C:\Users\<name>` patterns were already redacted.
* **Shared folders:** Apply always goes through a review panel; verdicts for OneDrive-synchronised paths are `Warning` (usable, with a stated caveat).
* **Full screen:** `host.toggleFullscreen()` reads the window's real state; the cached flag is only a display value, refreshed on the change event and (debounced) on resize.
* **Real-window smoke:** `scripts/desktop-smoke.mjs` drives the CDP-enabled build and `scripts/win-input.ps1` sends hardware-scan-code key events, moves/restores the window and reads `IsIconic`. The VLINUX window is found by title (`Process.MainWindowHandle` can return a WebView2 helper window).
