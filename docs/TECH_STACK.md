# Technology Stack — Candidates & Evaluation

**Status:** proposed choices only. Record selected versions, license audit, benchmark and platform tests before declaring production-ready.

| Layer | Candidate | Why | Caution / gate |
|---|---|---|---|
| Desktop shell | Tauri 2 | Native system bridge + web UI; packaging support | Native video embedding needs PoC |
| Frontend | React + TypeScript | Component-based multi-pane UX, typing | Control render/DOM memory |
| UI state | Zustand or equivalent | Simple local state store | Avoid duplicate registry source of truth |
| Terminal display | xterm.js | Proven terminal renderer | PTY backend and lifecycle separate |
| Local Windows shell | Windows ConPTY or supported PTY wrapper | Real interactive terminal | Win compatibility and UTF-8 handling |
| WSL | `wsl.exe` launched through PTY | Uses installed distribution | WSL availability and distro selection |
| Rust desktop core | Rust | Host integration, concurrency, process isolation | Rust alone does not guarantee speed |
| Linux host agent | Rust | Lightweight daemon candidate | Least privilege, deployment and upgrades |
| SSH | Rust SSH library or established SSH executable | Reliable transport, PTY, host-key trust | Library licensing and feature completeness |
| SFTP | SSH SFTP subsystem | Reuse authenticated SSH transport | Transfer cancellation/resume complexity |
| Remote desktop | FreeRDP / suitable VNC client backend | Mature protocols | Embedding, TLS, Wayland, legal/ABI checks |
| Android control | Google Android platform-tools (`adb`) | Official device transport | License, binary packaging, adb version conflicts |
| Android mirroring | scrcpy | Mature screen/input streaming | Embedded video and remote routing PoC |
| Agent control | SSH initially; versioned gRPC/WebSocket TBD | Reuse secure channel; structured events later | Authentication, data-plane design |
| Local persistence | SQLite | Zero-server, transactions, migrations | Encryption vs secret store separation |
| Credentials | Windows Credential Manager / platform keyring | Secrets outside SQLite plaintext | API support and secure migration |
| Discovery | mDNS / Zeroconf | LAN service discovery | Multicast limitations and spoofing risk |
| Future VPN | Tailscale / WireGuard | Encrypted reachability | No built-in VPN requirement for MVP |
| Testing | Rust tests, TS unit tests, Playwright where useful | Layered confidence | Hardware integration tests needed |

## Selection principles

1. Mature, actively maintained implementations and sustainable licensing.
2. Minimal resident overhead; on-demand heavy components.
3. Security posture (host key checks, ADB isolation, key stores).
4. Native integration and cross-platform API boundaries.
5. Clear package size and dependency update process.
6. Testability with mock adapters and real hardware.

## Deliberate boundaries

- No custom SSH, ADB, VNC, or RDP protocol implementation unless existing code cannot satisfy measurable needs.
- Flutter is an **external tool invoked on a configured build host**, not a framework used to build Senvakara.
- Tauri/Rust is for application core; React renders UI; scrcpy/ADB/backend libraries handle specialized streams.
- Platform compatibility target starts with Windows desktop and Linux host agent, then expands.

## Proof-of-concept checklist

- [ ] Tauri window with multiple terminals shows acceptable input/resize behavior.
- [ ] Rust PTY/SSH integration with proper host-key validation.
- [ ] SFTP operations and cancellation on Fedora.
- [ ] ADB list and `-s`-scoped operations for local Windows phone.
- [ ] Authorized ADB access to phone physically attached to Fedora without insecure exposed TCP service.
- [ ] scrcpy local and Fedora-remote trials with logged latency, CPU and bandwidth.
- [ ] RDP/VNC and Fedora Wayland compatibility tests.
- [ ] Idle/active whole-process memory and CPU measurements.
- [ ] Third-party dependency license and redistribution audit.

## Packaging concerns

Document whether Android platform-tools, scrcpy, remote desktop backend and any native DLLs are bundled, optional downloads, or external prerequisites; verify binary redistribution terms. Track platform versions and prevent multiple conflicting ADB servers from silently racing.

## Version policy

Pin actual versions in code manifests when bootstrapped, test dependency updates, and retain a compatibility matrix in `TESTING.md`. Avoid writing speculative exact package versions here.
