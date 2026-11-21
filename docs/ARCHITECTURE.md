# System Architecture — Senvakara

**Status:** proposed architecture, subject to proof-of-concept and ADR decisions.

**Update 2026-11-22 (ADR-023):** the desktop moved from Tauri 2 + React (v0.1.0, in git history) to a native Slint UI in 0.2.0. The layering is now:

```
apps/senvakara   Slint UI: .slint files draw view models; Rust controllers own all state and logic
crates/app       headless service layer (no UI toolkit): hosts, terminals, files, Android, mirroring,
                 remote desktop, workflows; session streams as callbacks, refresh hints on a broadcast channel
crates/*         adapters (ssh, adb, pty, scrcpy), storage, secrets, metrics
```

In the Slint app the pieces a web view used to provide are native: terminal emulation (`alacritty_terminal`), H.264 decoding for mirroring (`openh264`, ADR-024) and the VNC client (`vnc-rs` directly over the SSH `direct-tcpip` channel, so no WebSocket bridge).

## 1. Overview

```
Windows — Senvakara Desktop (native Slint UI + Rust core)
  ├─ UI: dashboard / sessions / files / workspaces / workflows
  ├─ Local host adapter: PowerShell / WSL / Windows metrics
  ├─ Local ADB: Android A via USB; Android C via paired Wi-Fi
  ├─ Registry: hosts, devices, connection routes, sessions
  ├─ Secure connection manager: SSH / tunnel / authenticated agent API
  └─ Protocol adapters: SSH+SFTP | RDP/VNC | ADB+scrcpy
                    │ secure Wi-Fi/Ethernet LAN, later VPN
                    ▼
Linux host (reference: Fedora) — SSH server + optional Senvakara Rust Agent
  ├─ Host metrics, heartbeat, device discovery
  ├─ Remote shell / file access (SSH/SFTP)
  ├─ Desktop service (compatible RDP/VNC backend)
  └─ Local ADB server → Android B via USB / authorized wireless ADB
```

Android uses existing ADB permission and debugging models; no custom phone-side agent required for baseline features. Other remote host OSes are future adapters.

## 2. Architectural boundaries

- **Desktop UI:** presentation, interactions, state. No secrets in frontend state or logs.
- **Rust desktop core:** sessions, application commands, connection routing, subprocess and lifecycle management.
- **Registry:** persisted logical hosts/devices/workspaces and their current *observed* routes; see `DATABASE.md`.
- **Connection manager:** verifies identity, maintains state and chooses secure transport; cannot equate availability with mere discovery.
- **Host agent:** reports capabilities/metrics/ADB devices, executes strictly authorized operations on the host; not an unrestricted remotely callable root shell.
- **Protocol adapters:** SSH/SFTP, desktop sessions, ADB/scrcpy; independent state, cancellation and errors.
- **Workflow engine:** planned DAG/sequential execution, validated targets, statuses and cancellation.

## 3. Host vs device vs route

A **host** is a compute node (Windows, Fedora). A **device** is a controllable target (including Android, and hosts represented as managed targets in the UI). A **route** describes *how* to reach a device from a particular host: Windows USB, Fedora USB, Windows Wireless ADB, Fedora Wireless ADB, SSH, or VPN path. Multiple routes may refer to the same physical Android. Stable **internal IDs** must not depend on IP, USB transport ID or display name.

No device auto-merge by model/name. If identifying material is incomplete, show separate route entries until user confirmation or verified match.

## 4. Control plane versus data plane

**Control plane:** host discovery, authenticated pairing, device listing, session creation, health, cancellation, command intent and progress. Use a versioned agent API *if selected*; secure SSH-based operations may be enough for early PoC.

**Data plane:** SSH PTY bytes, SFTP file streams, RDP/VNC pixel/input stream, scrcpy media/control streams, ADB protocol data and logcat. These may each require separate transports. **Do not force video through JSON WebSocket** without tests or simply forward insecure TCP.

## 5. Principal flows

### 5.1 SSH and SFTP to Fedora

1. User picks Fedora host.
2. Core resolves recorded SSH endpoint, checks host key / user credential.
3. Adapter opens PTY or SFTP channel.
4. Terminal/file UI streams data through typed events; cancellation closes channel.

### 5.2 Android B USB attached to Fedora, controlled from Windows

1. Fedora local ADB discovers authorized Android B (serial/transport binding).
2. Securely sent device event updates central registry: device route `fedora/usb/<transport>`.
3. User targets Android B in global device list (not host list).
4. Core resolves owner host and current verified route, then executes an allowlisted ADB action via authenticated agent or a protected ADB tunnel.
5. Display structured result; handle revoked debugging, USB disconnect, stale serial and lost host link.

**Never expose an unprotected remote ADB server on LAN/WAN.** Validate feasibility for remote scrcpy media separately in PoC, especially when ADB server resides on Fedora.

### 5.3 Android A (Windows USB) and Android C (Wi-Fi)

Windows ADB maintains local routes for authorized devices. Android C requires explicit Wireless Debugging pairing/connection and network reachability. A + B + C may be used concurrently with each action explicitly scoped to a host + ADB device selector.

### 5.4 Cross-host APK deployment

1. User selects build host, source and target Android; inspect permissions and supported capabilities.
2. Execute build via configured Flutter SDK on build host.
3. Stage artifact as needed over secure transfer (ideally streamed, bounded, verified).
4. Execute install via ADB service on *target's* host using explicit device selector.
5. Display status per stage; launch app/logcat only when user requested and previous steps passed.

### 5.5 Remote desktop

Use an existing RDP/VNC service on target host; Fedora desktop environment/Wayland backend requires compatibility testing. First demonstrate connection and keyboard/mouse. Embedded pane requires separate rendering benchmark (GPU/memory/latency) and native integration. Temporary external window must be labeled as such.

## 6. State machines (suggested)

**Host:** `unregistered → offline → connecting → online → degraded → reconnecting → offline`; separate `unauthorized` and `error` states.

**Route:** `discovered → authorized → available → busy/unavailable → expired`.

**Session:** `created → connecting → active → reconnecting → closed/failed`. Some sessions must be recreated, not resumed. Ordinary SSH shells do not survive disconnect unless remote persistence (e.g., tmux) is configured.

## 7. Modularity / internal contracts

Adapters should expose operations such as `capabilities`, `connect`, `create_session`, `stream`, `cancel`, `close`, with typed errors and explicit target descriptors. Never assume all adapters support file transfer, clipboard, recording or reconnect.

Use separate traits/interfaces for host discovery, terminal, files, desktop, Android and metrics rather than one oversized all-purpose adapter.

## 8. Performance model

Start only core, registry and UI. Spawn a terminal/SSH channel when requested; video/ADB stream only for active session; monitoring uses low-frequency scheduling; terminate children on close. Measure whole process tree (UI + agents + sidecars). Do not claim any idle budget achieved until benchmark.

## 9. Networking & trust

Initial LAN is **Wi-Fi or Ethernet**; peers need IP-level reachability and appropriate firewall rules. mDNS can help local discovery but cannot be assumed to work across subnets or VPN. Later VPN integrates at transport level with the same route abstraction. See `SECURITY.md` and `API.md`.

## 10. Reliability considerations

- Bounded queues/backpressure for logs/media and file transfers.
- Never replay non-idempotent commands after ambiguous disconnect without user confirmation.
- Detect route loss and invalidate stale device handles; unrelated sessions remain healthy.
- Versioned agent compatibility; staged upgrades and explicit unavailable-capability UI.
- Clean process shutdown, timeouts, retries with exponential backoff and cancellation.

## 11. Open feasibility gates

- Embedded RDP renderer and Fedora Wayland (VNC runs natively via `vnc-rs`).
- scrcpy stream embedding and reliable remote ADB routing.
- Windows local PTY API choice.
- Rust agent protocol and secure pairing.
- Identifier stability across Android USB and Wi-Fi.
- Dependency licensing and distributable binary sizes.

Document final selections in `DECISIONS.md`.
