# AGENTS.md — Instructions for AI Coding Agents

Applies to the entire Senvakara repository. **Before coding, read:** `README.md` → `PRD.md` → `ARCHITECTURE.md` → `SECURITY.md` → `TASKS.md`; then relevant `API.md`, `DATABASE.md`, and `TESTING.md`.

## Mission and product boundaries

Build Senvakara incrementally as a **local-first, device-centric control center**. Support Windows-local, WSL, Fedora remote and Android routed through local Windows ADB, Fedora-hosted ADB or authorized Wi-Fi ADB. Senvakara is **not** merely an app launcher, and must not require cloud services for initial capabilities.

## Non-negotiable agent rules

1. **Work from an existing task.** Choose a small unchecked item from `TASKS.md`. If user requests an unlisted feature, add/clarify task and acceptance criteria first.
2. **Do not claim success without verification.** Report actual command/test output, failed checks, and platform prerequisites. Documentation-only tasks should not pretend to build working code.
3. **Security first:** never expose ADB server port 5037, agent APIs, remote terminal or metrics unauthenticated on a public interface. Prefer SSH tunnels or authenticated encrypted channels. Don't disable host key verification to 'fix' connectivity.
4. **No secrets:** do not commit passwords, private keys, tokens, debug authorization material or logcat data; redact logs and examples. Use OS-backed secret stores when implemented.
5. **Least privilege:** Linux host agent runs unprivileged by default. No `sudo`/root instructions without a narrowly scoped, justified need. Dangerous device/system actions require explicit user initiation.
6. **Correct routing:** every Android command must bind to a concrete host and transport identifier; no global unqualified `adb shell` with multiple devices. Protect against stale mapping or accidental device substitution.
7. **Device identity != route identity.** Never merge devices solely by model/name/IP; require evidence or user confirmation if ambiguous.
8. **Lazy resources:** no idle video streaming, unnecessary polling, or background build processes. Tear down on close/cancel.
9. **Reuse protocols:** prefer established implementations for SSH, SFTP, RDP/VNC, ADB, scrcpy. New protocol implementations need a documented decision and tests.
10. **Keep decisions honest:** label proposed technology as candidate, not shipped. Do not invent benchmark figures or OS compatibility.
11. **Small, reviewable changes:** one cohesive concern per task; no unrelated mass refactors. Preserve public contracts or update dependent docs/tests.
12. **Test adverse paths:** offline host, disconnect during transfer, unauthorized ADB, different phones on two hosts, cancelled process, reconnect and failed pairing.

## Coding conventions (proposed; refine after repo bootstrap)

- Desktop (ADR-023): native Slint app in `apps/senvakara`. All state and logic in Rust; `.slint` files only draw view models and forward input; every colour and size comes from `ui/theme.slint`; no emoji or glyph icons; see `docs/DESIGN.md`. Backend calls go through `crates/app` and never block the UI thread.
- Fedora: standalone Rust agent/service with versioned protocol; no direct trust in incoming user-provided shell strings.
- Database: SQLite migrations, referential integrity, no raw secrets.
- Protocol adapters behind typed contracts; cancelable operations and structured errors.
- Prefer command execution by explicit executable + arguments, not shell interpolation. Whitelist or explicitly authorize operations that change device/host state.
- Keep UI responsive; long operations async, stream status, support cancel.
- Separate application logs from raw SSH/logcat output, default redaction and retention controls.

## Task execution workflow

1. **Understand:** cite relevant requirement ID (e.g., `AND-03`) and its acceptance criteria.
2. **Design:** identify trust boundary, host/route selection, failure states, resource lifecycle.
3. **Implement:** smallest vertical slice with tests and migrations where needed.
4. **Validate:** formatting/lint/unit tests; integration tests where hardware available; state what wasn't tested.
5. **Document:** update `TASKS.md` checkbox only when verifiably done; update `DECISIONS.md` for architectural trade-offs and `CHANGELOG.md` for user-visible changes.
6. **Summarize:** files changed, behavior, actual tests, limitations, next task.

## Suggested checks after bootstrap (verify tool availability)

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
```

These are **illustrative**, not guaranteed existing scripts. Never report them as passed unless run.

## Definition of done

- Acceptance criteria met with an observed test or appropriate automated test.
- Errors/offline states handled, device targeting explicit, credentials not leaked.
- Related docs and migration/versioning updated.
- No accidental resource/process leaks and no unauthorized network exposure.

## Critical user journey

**Windows DevHub desktop** simultaneously sees **Android A on Windows USB** + **Android B on Fedora USB**; later it sees **Android C via authorized Wi-Fi ADB**, and also supports terminal/SSH/SFTP/RDP and saved workspaces. Do not collapse Senvakara into Android-only tooling.
