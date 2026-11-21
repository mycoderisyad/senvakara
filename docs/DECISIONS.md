# Architecture Decisions — Senvakara

This is a lightweight ADR (Architecture Decision Record) log. **Accepted** describes a design direction chosen in brainstorming, *not* a tested working implementation. Pending selections must remain explicit.

## Decision index

| ID | Status | Decision | Rationale |
|---|---|---|---|
| ADR-001 | Accepted (product) | Product name: Senvakara | Chosen working name; legal/trademark review outstanding |
| ADR-002 | Accepted (product) | Unified device-centric, host-aware model | Android can be attached to any authorized host |
| ADR-003 | Accepted (product) | LAN-first (Wi-Fi/Ethernet), future VPN | Feasible initial deployment without mandatory cloud |
| ADR-004 | Accepted (direction) | Local-first, lightweight, modular, secure by default | Privacy and manageable performance |
| ADR-005 | Proposed / PoC pending | Tauri 2 + React + TypeScript + Rust desktop core | Candidate for UI + native integrations |
| ADR-006 | Proposed / PoC pending | Linux Rust agent and secure agent control API | For inventory/metrics/ADB bridge; may start with SSH |
| ADR-007 | Accepted (direction) | Reuse existing SSH/SFTP/RDP/VNC/ADB/scrcpy technology | Reduced maintenance/risk vs rewriting protocols |
| ADR-008 | Proposed / PoC pending | Embed RDP/VNC and scrcpy inside Senvakara UI | Ideal UX; renderer feasibility unresolved |
| ADR-009 | Accepted (direction) | SQLite for metadata; secrets in OS-protected store | Local-first storage with secret separation |
| ADR-010 | Proposed / security review pending | Agent transport and pairing design | Must be authenticated/encrypted and versioned |
| ADR-011 | Accepted (product) | Multiple devices/connection routes per host | Windows USB, Fedora USB and Wireless ADB coexist |
| ADR-012 | Accepted (plan) | MVP SSH/SFTP/local terminal + cross-host basic ADB | Prove differentiation before video complexity |
| ADR-013 | Accepted (product) | Remote hosts are generic Linux; Fedora is only the reference test host | No distro-specific package managers/paths; host capabilities detected at connect time (`/etc/os-release`, `uname -m`, `adb`, session type, desktop environment) |
| ADR-014 | Accepted (PoC evidence POC-05) | Remote ADB via SSH local port-forward to the Linux host's `127.0.0.1:5037` (no agent required for MVP) | Reuses verified SSH transport; ADB server never leaves localhost |
| ADR-015 | Accepted (licensing) | Do not bundle Google's prebuilt `adb.exe`; detect user install or offer official download with user acceptance of Google terms | Android SDK License §3.4 restricts redistribution |
| ADR-016 | Accepted for M1 (PoC evidence POC-02/03/04) | Libraries: `portable-pty` (ConPTY), `russh` + `russh-sftp`, `rusqlite`, `keyring` 4.x, `@xterm/xterm` 6.x; binary streams via Tauri `Channel` raw bodies | Mature, permissive licenses (MIT/Apache-2.0); pin exact versions; russh uses `ring` backend (aws-lc-sys needs NASM on Windows) |
| ADR-017 | Superseded by ADR-019 (mirroring part) | Embedded mirroring via scrcpy-server + `@yume-chan/scrcpy` + WebCodecs; embedded RDP via IronRDP in Rust core; VNC via noVNC + loopback WS bridge | Webview cannot open raw TCP; scrcpy server/client versions must match exactly; external viewers remain labeled fallbacks. Evidence: H.264 WebCodecs decode works in WebView2 157 on the reference client (TESTING.md §9) |
| ADR-018 | Accepted (PoC evidence) | Speak the ADB host protocol directly over TCP (`crates/adapters-adb`) instead of spawning `adb.exe -H/-P` per command | Streaming `track-devices` without polling; actions bound to transport ids so reconnects surface as `ROUTE_STALE`; a version-mismatched local client can never kill a remote server. `adb.exe` is only used to start the local server |
| ADR-019 | Accepted (PoC evidence) | Own minimal scrcpy client in Rust (`crates/adapters-scrcpy`) talking to a bundled, SHA-256-pinned `scrcpy-server` v3.3.3 via `localabstract:` over the ADB host protocol; H.264 decoded with WebCodecs into a canvas | Works unchanged for phones behind a Linux host tunnel; no extra JS protocol library or version coupling to a third-party client; Apache-2.0 server bundled with notice; input sent in order (UI queue) because separate invokes can be reordered |
| ADR-020 | Accepted | No resident Linux agent for now: metrics, device watch and desktop launch use the authenticated SSH connection (read-only `/proc`, ADB tunnel, on-demand x11vnc) | Smallest attack surface and nothing to install or update on hosts; revisit if push events or non-SSH transports are needed (ADR-006 stays open) |
| ADR-021 | Accepted (implementation; host test pending) | Remote desktop follows the target: text-mode host → terminal only; active X11 session → share it with x11vnc on a private Unix socket, bridged to noVNC through a one-time loopback WebSocket; Wayland → reported unsupported | Matches the user's requirement; no listening TCP port on the host; no extra trust material |

## ADR template for new choices

```md
### ADR-XXX — Short title
- Date:
- Status: Proposed | Accepted | Superseded | Rejected
- Context:
- Decision:
- Alternatives:
- Trade-offs / risks:
- Validation evidence:
- Security/compatibility/licensing implications:
- Related files/tasks:
```

## Open decisions and evidence needed

1. Which Windows PTY implementation and SSH library pass terminal and security tests?
2. Does remote ADB through Fedora work best via SSH tunnel, agent RPC or both? Measure reliability and avoid exposing port 5037.
3. Which protocol stack reliably embeds scrcpy and RDP/VNC in Tauri without unacceptable overhead? Benchmark RAM, CPU, latency.
4. What Fedora Wayland desktop protocol/server stack is supported?
5. Which encrypted, authenticated agent transport/pairing scheme is practical and secure?
6. How can the same Android be verified across USB and wireless route changes without accidental merge?
7. Which third-party binaries are legally distributable/bundleable, and what is final project license?
8. Should future VPN connectivity be user-managed or have optional guided setup?

When a decision changes, add superseding ADR and update impacted `ARCHITECTURE.md`, `TECH_STACK.md`, `API.md`, `SECURITY.md`, `TASKS.md` rather than silently rewriting history.
