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

### Application

- No application release yet. Implementation pending.

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
