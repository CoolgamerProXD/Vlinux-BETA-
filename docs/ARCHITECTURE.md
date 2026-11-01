# VLINUX architecture (M0 + M1)

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

Planned (later milestones): `wsl` (detection/management behind a backend trait so non-WSL backends can be added), `pty` (ConPTY/`wsl.exe` terminal sessions), `fs`, `distro` (registry + installer).

### Frontend modules
* `lib/wm.svelte.ts` — floating shell-window manager with workspaces (unit-tested).
* `lib/config.svelte.ts` — reactive config, applied live, saved debounced via Rust.
* `lib/host.svelte.ts`, `lib/clock.svelte.ts` — event-driven; **no polling timers**. The clock wakes only on minute (or second) boundaries.
* `lib/apps.ts` — app registry. Apps with `status: 'planned'` render disabled and labelled with their milestone, so there are no buttons that do nothing.
* Wallpapers are generated procedurally from a seed (`wallpaper-gen.ts`): original, tiny, resolution independent.

## Config schema (TOML)
```toml
schema_version = 1
[shell]      # start_fullscreen, clock_24h, show_seconds, show_date, workspace_count (2-9)
[appearance] # wallpaper = nebula|dusk|peaks|grid, accent = violet|cyan|rose|amber|mint, glass 0-100, animations
[dock]       # position = bottom|left, icon_size 32-72
```
`src/lib/types.ts` mirrors `config.rs` (including clamping rules).

## Decisions / deviations from the spec
* Stack is as specified (Rust, Tauri 2, Svelte 5 + TS, CSS). xterm.js is deferred to M3.
* Fonts (Inter, JetBrains Mono) are bundled via npm so the UI works offline; CSP is `default-src 'self'`.
* NSIS (per-user) is the only installer target, so no admin prompt is needed to install or uninstall VLINUX.
* Single-instance enforcement is not implemented yet (would add a plugin dependency); listed as a limitation.
