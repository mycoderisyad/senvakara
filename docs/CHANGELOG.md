# Changelog — Senvakara

Changes to documentation and (later) shipped product are tracked separately. Format inspired by Keep a Changelog; no released binary is implied.

## [Unreleased]

### Documentation — 2026-11-21

- Established project name **Senvakara** (working name, availability checks pending).
- Split initial product brainstorming into 14 Markdown files.
- Defined device-centric multi-host architecture: Windows USB, Fedora USB and authorized Wi-Fi ADB in one registry.
- Documented SSH, SFTP, local/WSL terminal, RDP/VNC, scrcpy, monitoring, workspaces, cross-host Flutter deployment and automation.
- Added phased roadmap, MVP boundary, security requirements, proposed SQLite model, API contract, tests and AI coding agent rules.

### Documentation — 2026-11-21 (planning update)

- Generalized remote host target from Fedora to generic Linux; Fedora remains the reference test host (ADR-013).
- Added ADR-014..017: remote ADB via SSH port-forward, no bundled `adb.exe`, candidate library set, embedded streaming approach.

## [0.2.0] - 2026-11-22 (built, not yet signed off)

Installer `senvakara_0.2.0_x64-setup.exe` (NSIS, per-user) is built but has not been run; install, upgrade from 0.1.0 and uninstall are untested.

### Changed

- The desktop app is now a native Slint application (`apps/senvakara`, ADR-023). The Tauri + React app of 0.1.0 was removed; it remains in git history.
- Navigation: an inventory Home (Hosts, Devices, Workspaces, Automations, Settings) with an inspector, and every session as a tab: terminals (PowerShell, WSL, SSH, tmux), Android screen, logcat, dual-pane files (PC, host, phone), remote desktop, workspaces, automation runs. Command palette on Ctrl+Shift+K.
- Backend moved into the UI-independent crate `crates/app`. Only one Senvakara instance per data folder can run.
- Terminal emulation (`alacritty_terminal`), H.264 decoding (`openh264`, ADR-024) and the VNC client (`vnc-rs` on the SSH channel) are native; WebView2 and Node.js are no longer needed.
- Idle memory about 51 MB private (0.1.0: 204 MB); startup 0.14–0.50 s.

### Known gaps

- Many flows were exercised only up to their confirmation, or not on hardware (a phone behind the Linux host, two mirrors, Flutter deploy, recipe steps other than wait). See TESTING §9 and TASKS Phase 8.
- Idle CPU on Home is higher than 0.1.0 (0.73 % of one core against 0.08 %).
- Folder copy in Files, a forced-dark title bar, Tight/JPEG rectangles in remote desktop and the WebCodecs diagnostics page (replaced by a decoder self-test) are not there.

## [0.1.0] - 2026-11-21

First Windows installer (`Senvakara_0.1.0_x64-setup.exe`, NSIS, per-user, no admin rights; not code-signed, so Windows SmartScreen may warn). Third-party notices ship in the install folder.

### Added

- Desktop shell (Tauri 2 + React + TypeScript) with sidebar navigation; unbuilt sections state their planned milestone.
- Local terminal tabs: Windows PowerShell and each installed WSL distribution (ConPTY), resize, close kills the process.
- Diagnostics view: WebCodecs codec support and H.264 round-trip check for the local WebView2.
- Hosts: add/edit/remove Linux hosts, first-connect host-key verification dialog, changed-key warning, connect/disconnect, detected OS/arch/ADB shown per host.
- SSH terminal tabs alongside local tabs.
- Files: browse remote folders over SFTP, upload/download with progress, cancel, and confirmation before replacing files.
- Key passphrases stored in Windows Credential Manager; local SQLite stores only metadata.
- Redacted application log in the user's local app-data folder.
- Devices: Android phones from this PC and from connected Linux hosts in one list, labelled with host and route; Info and confirmed one-shot Shell bound to the exact device; rename/forget; live updates without polling.
- Device page: screenshot (save as PNG), streaming logcat with filter, user-app list with APK install/uninstall, device file browser with push/pull — every mutating action confirms the exact device and route.
- Live screen mirroring with touch, scroll, keyboard and device buttons for phones on this PC and on connected Linux hosts (bundled scrcpy-server v3.3.3, hash-verified).
- Add phones over Wi-Fi (Android 11+ Wireless debugging): network scan, pairing code, connect/disconnect, through this PC or a connected Linux host.

- Overview dashboard: live CPU, memory, disk, network and uptime for this PC and connected Linux hosts.
- Hosts show whether the host has a graphical session; Desktop shows the host's X11 session in a Senvakara pane with mouse and keyboard (requires x11vnc on the host; loopback-only, one-time password, stopped on close).

- Automatic reconnect of hosts after a connection loss (bounded backoff, stops on authentication problems or when you disconnect).
- tmux-backed SSH tabs that survive disconnects and reattach with ↻.

- Workspaces: save and reopen 1/2/4-panel layouts mixing terminals (incl. tmux), phone screens, logcat, files and remote desktops; panels that cannot run explain why and offer to connect the host.

- Send files straight from a Linux host to any phone, or from a phone to a Linux host, with progress and a size check — nothing is stored on this PC.

- Deploy: build a Flutter app on this PC and install it on any phone (including phones on another host), optionally launch it and show its log; step-by-step status, cancel, and run history.

- Network paths: give a Linux host extra VPN addresses (WireGuard, Tailscale, … that you already run; addresses seen on the host are offered with one click). Senvakara uses the LAN address when it can and falls back to a VPN address automatically when you are away, always requiring the same trusted host key; phones behind the host keep their identity, and ADB is never exposed.

- Automations: save recipes of steps (install/uninstall APK, launch app, wait, shell command, screenshot, Flutter build & install) for one phone, with explicit per-recipe permissions; every run shows all steps and the exact target for confirmation, stops at the first failure, can be cancelled and is kept in history.

### Changed

- Release builds unwind on panic, so a failure in one session does not close the whole app (NFR-02).
- Placeholder Senvakara app icon instead of the framework's default.

### Fixed

- Device availability shown after a restart no longer reflects the previous session.

## Versioning guidance

When the app is released, use entries such as:

```md
## [0.1.0] - YYYY-MM-DD
### Added
- ...
### Changed
- ...
### Fixed
- ...
### Security
- ...
```

Do not fabricate versions, release dates, benchmark results or working features.
