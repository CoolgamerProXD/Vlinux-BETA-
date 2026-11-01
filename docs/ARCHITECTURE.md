# VLINUX architecture (M0 – M2)

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

### Storage architecture (groundwork; mappings are applied from M4)
Defaults: `~/Downloads`, `~/Documents`, `~/Pictures`, `~/Videos`, `~/Music`, `~/Desktop` → the matching Windows Known Folder, accessed directly through the drive mount (no copy, no sync). Linux system files, package databases, caches and dotfiles are always private to the distribution. Development projects default to **Linux-native** storage (recommended; `/mnt` access is slower) and can be switched.
* Known Folders are resolved dynamically via the shell folder API (`dirs` → `SHGetKnownFolderPath`), so redirected folders and other drives work; no user name, `C:\Users` path or drive letter is hard-coded.
* `windows_to_wsl(path, mount_root, drives)` is pure and tested. It rejects UNC (`\\server\share`, `\\?\UNC`), `\\wsl$` / `\\wsl.localhost`, device paths, drive-relative paths, `..`, illegal characters, reserved names, trailing space/dot and bare drive roots, and **warns** for network and removable drives (`GetDriveTypeW`). The mount root is assumed to be `/mnt`; reading `/etc/wsl.conf` / `wslpath` is an M4 follow-up.
* `plan_mapping(mode, verdict, existing_linux_entry)` is a pure planner that can never plan data loss: a missing path → create link; the same link → no-op; a symlink elsewhere → retarget (confirmation); an **empty** directory → replace (confirmation); a **non-empty directory or a file → handed to the user, never changed**; unsupported target → blocked; switching to private only unlinks a symlink (confirmation) and leaves the Windows files untouched.
* UI: Settings → *Linux folders* shows, per folder, the Linux path, where it points (or why it cannot be shared), a share/private switch, an alternative Windows folder with a *Check* step, a projects selector and *Restore defaults*. All of it is labelled as saved preferences that are not applied yet.

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
