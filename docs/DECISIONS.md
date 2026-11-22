# Architecture Decisions — Senvakara

This is a lightweight ADR (Architecture Decision Record) log. **Accepted** describes a design direction chosen in brainstorming, *not* a tested working implementation. Pending selections must remain explicit.

## Decision index

| ID | Status | Decision | Rationale |
|---|---|---|---|
| ADR-001 | Accepted (product) | Product name: Senvakara | Chosen working name; legal/trademark review outstanding |
| ADR-002 | Accepted (product) | Unified device-centric, host-aware model | Android can be attached to any authorized host |
| ADR-003 | Accepted (product) | LAN-first (Wi-Fi/Ethernet), future VPN | Feasible initial deployment without mandatory cloud |
| ADR-004 | Accepted (direction) | Local-first, lightweight, modular, secure by default | Privacy and manageable performance |
| ADR-005 | Superseded by ADR-023 (2026-11-22) | Tauri 2 + React + TypeScript + Rust desktop core | Shipped as v0.1.0; removed in 0.2.0 |
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
| ADR-021 | Accepted (tested on reference host) | Remote desktop follows the target: text-mode host → terminal only; active X11 session → x11vnc on demand, loopback-only random port (verified with `ss` after start), one-time password via SFTP into the 0700 runtime dir (`-passwdfile rm:`), one client, stopped on close; noVNC through a one-time-token loopback WebSocket bridge over SSH `direct-tcpip`; Wayland → reported unsupported | First design (x11vnc `-unixsock` + `-rfbport 0`) failed: unixsock needs a patched libvncserver and `-rfbport 0` still listened on `[::]:5900` without a password — caught in testing, processes stopped, guard added |
| ADR-022 | Accepted (tested on reference host) | VPN = extra *paths* to the same host, never a new identity. A host may have up to 4 user-managed VPN addresses (WireGuard, Tailscale, …; Senvakara creates no VPN). Connect probes all paths in parallel, uses the most preferred reachable one (LAN first), gives a path that accepts TCP but stalls 5 s when others remain, and requires the same pinned host key on every path (checked before credentials are sent). Device routes stay bound to the host, so identities survive LAN↔VPN switches. ADB stays loopback-only inside SSH | Answers open decision 8 (user-managed VPN with guided registration: VPN interfaces seen on the host are offered as one-click addresses). Rejected: per-address trust (an impostor at an old LAN address could be trusted by mistake), exposing ADB/agent ports on the VPN |
| ADR-023 | Accepted (2026-11-22, owner decision after the spikes; the Tauri app was removed in 0.2.0 before full parity sign-off, see TASKS UI-MIG-11) | Replace the Tauri 2 + React UI with a native Slint UI (`apps/senvakara`; pinned to Slint 1.18.1 with `vtable` 0.5.0 because Slint 1.19.0 needs rustc 1.95 and the project toolchain is 1.94.1) on top of a Tauri-free service crate (`crates/app`). Terminal emulation moves to `alacritty_terminal`, the VNC client to `vnc-rs` over SSH `direct-tcpip`, file dialogs to `rfd`. The Tauri app was removed on the owner's instruction on 2026-11-22; v0.1.0 remains in git history (commit 23bff18). Supersedes ADR-005 and the rendering parts of ADR-016/017/019/021 | WebView2 alone costs about 200 MB private memory, so the idle-RAM budget cannot be met (TESTING section 6); the React UI was judged unprofessional by the owner; one language and one toolchain. Slint is used under its Royalty-free License 2.0, which requires visible attribution (`AboutSlint` in Settings). Risk: everything the web view provided (terminal, H.264 decode, VNC client) must be rebuilt natively |
| ADR-024 | Accepted (owner decision 2026-11-21; revisit before wide distribution) | Decode the scrcpy H.264 stream in Rust with `openh264` built from source (BSD-2-Clause) instead of WebCodecs | Simplest portable option with no extra runtime download. Open risk: Cisco's patent coverage applies only to Cisco's own prebuilt binary downloaded by the end user, not to a source build, so H.264 patent licensing must be reviewed before the app is distributed widely. Software decoding may cost more CPU than the GPU path WebCodecs used; Windows Media Foundation is the fallback candidate |
| ADR-025 | Accepted (2026-11-22, owner decision) | Keychain storage: secrets (SSH passwords, private keys, key passphrases) live only in Windows Credential Manager through `crates/secrets`. A blob entry is limited to `CRED_MAX_CREDENTIAL_BLOB_SIZE` = 2560 bytes, so larger secrets are split into chunks of at most 2 KB plus a manifest entry. Non-secret metadata (label, algorithm, public key, certificate, opaque references) is kept in SQLite (`keychain_keys`, migration 0005) | RSA 3072/4096 private keys exceed one entry; the database must never hold secrets (SECURITY §5). Rejected: encrypting keys into SQLite with an app-held key (the key would need its own storage) and keeping only key file paths (no import, breaks when the file moves) |
| ADR-026 | Accepted (2026-11-22, owner decision) | RDP = launch `mstsc.exe` with explicit arguments (no shell); the credential is written to Credential Manager under the target `TERMSRV/<address>` before launch and removed when the host is deleted. Embedded RDP is rejected for now. Partial answer to DESK-01 (RDP by external client; VNC stays embedded) | Reuses the Windows client with NLA, clipboard and audio at no protocol-implementation cost (AGENTS rule 9); a Rust RDP client would need its own decision and tests. Trade-off: not embedded in a Senvakara tab. Reopen if an embedded client becomes necessary (DESK-IMPL-01) |
| ADR-027 | Candidate (untested without hardware) | FIDO2 security keys through the Windows OpenSSH agent: connect with russh `AgentClient::connect_named_pipe` to `\\.\pipe\openssh-ssh-agent` and authenticate with the agent's keys, so Senvakara never handles the hardware key | Reuses the OS agent instead of speaking CTAP/HID; depends on the user running the Windows OpenSSH agent with an enrolled `sk-` key. Not shipped or tested until a security key is available (TASKS CRED-04) |

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
7. Which third-party binaries are legally distributable/bundleable? (Project license decided: Apache-2.0, see `LICENSE`.)
8. ~~Should future VPN connectivity be user-managed or have optional guided setup?~~ Decided in ADR-022: user-managed, with guided address registration.

When a decision changes, add superseding ADR and update impacted `ARCHITECTURE.md`, `TECH_STACK.md`, `API.md`, `SECURITY.md`, `TASKS.md` rather than silently rewriting history.
