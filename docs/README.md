# Senvakara

> **One Application. Every Device. Unified Control.**

Senvakara is a proposed **local-first unified remote access and development control center**. Its Windows desktop client will manage local and remote computers, WSL environments, and Android devices attached by USB or authorized wireless debugging—even when the Android device is physically attached to another managed computer.

**Status:** development builds only. Milestones M0–M7 are implemented and verified on the reference hardware (see [TASKS.md](TASKS.md) evidence and [TESTING.md](TESTING.md) §9); RDP and VPN routes (M8) are not built. There is **no released application** yet and no benchmarked performance results.

## Reference setup

> **Terminology:** "Fedora" in these documents means the *reference Linux test host*. Senvakara targets generic Linux remote hosts (see ADR-013); behavior must not depend on a specific distribution.

- Windows laptop: primary desktop client; local PowerShell/terminal, WSL, and USB Android devices.
- Fedora laptop: remote Linux host, initially reached on the same Wi-Fi/Ethernet LAN; can expose its own terminal, desktop, SFTP, metrics, and USB-attached Android devices through authenticated mechanisms.
- Android devices: may be attached via USB to **either host**, or paired for **ADB over Wi-Fi**. More than one can be managed concurrently.
- Future: additional Linux/Windows hosts and connectivity across different networks using a user-managed VPN (e.g., Tailscale/WireGuard).

## Guiding idea

Choose a **device and an action**, not which remote machine or ADB command to navigate manually. A global registry links devices to their hosts and available connection routes.

## Documentation map

| File | Purpose |
|---|---|
| [PRD.md](PRD.md) | Product problem, users, requirements, acceptance criteria, scope |
| [AGENTS.md](AGENTS.md) | AI coding agent working rules and change workflow |
| [ARCHITECTURE.md](ARCHITECTURE.md) | System boundaries, control/data planes, flows, failure behavior |
| [TECH_STACK.md](TECH_STACK.md) | Candidate technologies, trade-offs, validation status |
| [DESIGN.md](DESIGN.md) | UX behavior, visual design tokens, navigation, states |
| [TASKS.md](TASKS.md) | Actionable staged backlog with checkboxes |
| [ROADMAP.md](ROADMAP.md) | Milestones and exit gates |
| [DATABASE.md](DATABASE.md) | Initial SQLite logical schema and migration plan |
| [API.md](API.md) | Proposed agent protocol contracts and versioning |
| [TESTING.md](TESTING.md) | Test matrix, acceptance tests, perf/security checks |
| [SECURITY.md](SECURITY.md) | Threat model, trust boundaries, credential/device protections |
| [DECISIONS.md](DECISIONS.md) | Architecture decision record log, decisions vs. proposals |
| [CHANGELOG.md](CHANGELOG.md) | Documentation and future release history |

## Scope and implementation order

1. **Proof of concept:** SSH, local terminals, SFTP, Android connected to Fedora reached from Windows over a protected route.
2. **MVP:** desktop shell, device/host registry, terminal and SFTP, basic ADB commands routed to Android on Windows and Fedora.
3. **Post-MVP:** wireless ADB onboarding, mirroring, RDP/VNC integration, workspaces, monitoring and deployment orchestration.
4. **Later:** recipes, sophisticated recovery, secure cross-network VPN paths, additional host platforms.

See [ROADMAP.md](ROADMAP.md) for explicit gates. This ordering is intentional: embedded video and remote-desktop sessions are **not** assumed to be solved in the MVP.

## Setup / running

Prerequisites (Windows): Rust stable (MSVC), Node.js 24, WebView2 runtime, and for Android features the user's own Android platform-tools (`adb`, not bundled — ADR-015).

```powershell
cd apps/desktop
npm install
npm run tauri dev      # run the desktop app
npm run typecheck; npm run lint; npm run test
cargo test --workspace # from the repository root
```

Linux host PoC spike: `cargo run -p senvakara-adapters-ssh --example poc_linux -- <host> <user> <SHA256:host-key-fingerprint> [private-key]`.

Other tools used later include Rust, Tauri 2, Node.js, Android platform-tools, scrcpy, Flutter SDK (only for optional deployment workflows), SSH server on Fedora, and appropriate desktop-sharing services.

## Documentation rules

- Product intent: `PRD.md`; system boundaries: `ARCHITECTURE.md`; protocols: `API.md`; schema: `DATABASE.md`.
- Mark uncertain technology choices as **candidate / pending PoC**; don't imply they are already implemented.
- See `AGENTS.md` before editing code or requirements.
- Use `DECISIONS.md` for nontrivial technical choices and `CHANGELOG.md` for user-visible changes.

## Naming and licensing

**Senvakara** is a working project name; trademark, repository name, domain, and third-party license checks remain open. Project license and distribution policy are not yet decided.
