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

### Application (development builds only, no release)

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
