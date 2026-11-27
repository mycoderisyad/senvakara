# Product Requirements Document — Senvakara

**Version:** 1.0 (planning)  
**Date:** 2026-11-21  
**Status:** Draft. Requirements are targets, not completed features.

## 1. Product summary

Senvakara is a lightweight, local-first **unified remote access and development control center** for computers, development environments, and Android devices. From one desktop window, users should be able to operate SSH, SFTP, terminal, remote desktop (RDP/VNC), Android ADB/scrcpy, monitoring, and cross-device workflows without constantly switching tools.

**Vision:** One Application. Every Device. Unified Control.

## 2. Problem and differentiation

A developer using Windows + WSL + Fedora + physical Android devices currently needs several independent tools. Connections, sessions, and device context are fragmented. Senvakara's intended differentiation is **topology-aware orchestration**, not inventing better versions of SSH, RDP or ADB. Examples:

- An Android physically attached via USB to Fedora appears as a first-class device inside Senvakara running on Windows.
- Android devices attached directly to Windows, to Fedora, and paired over Wi-Fi are visible at once.
- A Flutter build on one host can be deployed to an Android reachable through another authorized host.
- Saved workspaces group terminal, file explorer, desktop, Android, and logs in a common layout.

This is an aspiration relative to existing tools—not an assertion of competitive superiority.

## 3. Users and jobs to be done

| User | Core job |
|---|---|
| Individual developer | Use Windows/WSL and SSH to Fedora without changing apps |
| Android/Flutter developer | Discover, deploy and debug multiple physical phones on different hosts |
| Homelab user / sysadmin | Securely manage computers, files, sessions and health from one place |
| Future QA/team user | Reuse workspaces and recipes across test devices (later) |

## 4. Core principles

1. **Device-centric, host-aware:** a device has a stable internal identity; a connection is merely a route to it.
2. **Local-first:** no account, cloud, or remote relay needed for baseline operation.
3. **Network-agnostic:** same application semantics on local Wi-Fi, wired Ethernet, or later a VPN.
4. **Secure by default:** explicit trust, identity verification, encryption and least privilege.
5. **On-demand and modular:** start heavy subprocesses, streams, and sessions only when needed.
6. **Reuse mature technology:** don't recreate SSH, ADB, scrcpy, RDP or VNC without measured reason.
7. **Unified UX:** integrated tabs/panels are the long-term target; an external window is acceptable only as a clearly labeled interim fallback.

## 5. Reference deployment and boundaries

- Windows: desktop control center, Windows terminal, WSL, local ADB server, USB phones.
- Linux host (any distribution; reference test host: Fedora): SSH host and optional authenticated lightweight Rust agent; its own ADB server and USB phones; remote desktop where configured.
- Android: accessible only via authorized ADB debugging (USB or Wireless Debugging where supported).
- Initial networking: same LAN over Wi-Fi or Ethernet, and manually registered IP/hostname. Guest Wi-Fi isolation/firewalls can prevent connectivity.
- Later networking: VPN or equivalent secure connectivity to remote networks; no requirement to expose ADB directly to the internet.

## 6. Functional requirements

Legend: **P0** MVP requirement; **P1** target before stable 1.0; **P2** future enhancement. IDs remain stable for tests and tracking.

### 6.1 Inventory, connection & host control

| ID | Priority | Requirement |
|---|---|---|
| INV-01 | P0 | Add Windows-local and Fedora-remote hosts; save display name and connectable address. |
| INV-02 | P0 | Global registry lists hosts and Android devices separately, including origin host and current route. |
| INV-03 | P0 | Clearly show online, offline, connecting, unauthorized and error states. |
| INV-04 | P0 | Maintain host/device identity independently from IP and transport endpoint. |
| INV-05 | P1 | Secure host-agent pairing, heartbeat and local-network discovery (with manual fallback). |
| INV-06 | P1 | Manage multiple host connections concurrently and expose device capabilities. |
| INV-07 | P2 | Multiple verified routes per device, automatic selection/failover without accidental identity merges. |

### 6.2 Terminal and file management

| ID | Priority | Requirement |
|---|---|---|
| TERM-01 | P0 | Run Windows PowerShell/local command sessions and WSL shell. |
| TERM-02 | P0 | Interactive SSH sessions to Fedora with host-key verification and key-based auth. |
| TERM-03 | P0 | Multiple tabs, resize, input, copy/paste and explicit termination. |
| TERM-04 | P1 | Split views, saved profiles and reconnection behavior; optional tmux reattach. |
| FILE-01 | P0 | Browse remote directories and upload/download through SFTP. |
| FILE-02 | P1 | Local/remote file browser, transfer progress, cancellation, failures and Android push/pull. |
| FILE-03 | P1 | Cross-device transfer orchestrated by appropriate trusted hosts. |
| FILE-04 | P2 | Queues, resume/retry and integrity validation where supported. |

### 6.3 Remote desktop

| ID | Priority | Requirement |
|---|---|---|
| DESK-01 | P1 | Connect to supported RDP and VNC endpoints (with proper authentication). |
| DESK-02 | P1 | Keyboard/mouse control, resizing, session termination, error states. |
| DESK-03 | P1 | Render sessions within the Senvakara window after embedded-backend PoC; interim external viewer explicitly labeled. |
| DESK-04 | P2 | Multiple side-by-side remote desktops, clipboard permissions and adaptive quality. |

### 6.4 Android from any authorized host

| ID | Priority | Requirement |
|---|---|---|
| AND-01 | P0 | Detect USB Android on Windows via local ADB. |
| AND-02 | P0 | Detect USB Android on Fedora from Windows via authenticated/tunneled route. |
| AND-03 | P0 | List both simultaneously; route each ADB action to correct host and transport. |
| AND-04 | P0 | Basic ADB action proof: device info / shell / authorized command with visible result. |
| AND-05 | P1 | Pair/connect supported Android Wireless Debugging over reachable Wi-Fi (with user approval). |
| AND-06 | P1 | scrcpy-based mirroring and user input, local and Fedora-attached devices. |
| AND-07 | P1 | Install/uninstall APK, logcat, shell, screenshot and Android file push/pull. |
| AND-08 | P1 | Multiple independent sessions and explicit targeting by device identity. |
| AND-09 | P2 | Route fallback and verified identity reconciliation after switching between USB and Wi-Fi. |

**Limit:** Sharing Wi-Fi is not sufficient by itself. Android must support, enable, pair and authorize debugging; older Android versions may require initial USB setup. Never bypass platform security.

### 6.5 Monitoring, workspace and workflows

| ID | Priority | Requirement |
|---|---|---|
| MON-01 | P1 | Show host CPU, RAM, storage, uptime, network metrics with adjustable sampling. |
| WORK-01 | P1 | Save named workspaces with linked hosts, Android targets, tabs and split layout. |
| WORK-02 | P1 | Restore layout and reconnect eligible sessions; explain unrecoverable sessions. |
| FLOW-01 | P1 | Select build host and Android target; launch approved Flutter build command. |
| FLOW-02 | P1 | Transfer built APK to destination host and install/launch on target Android. |
| FLOW-03 | P1 | Stream logs and step statuses; expose errors and stop/cancel behavior. |
| FLOW-04 | P2 | Save reusable automation recipes and history with explicit permissions. (Since WF-IMPL-01, recipes are workflows on the Workflows canvas.) |
| REC-01 | P1 | Detect disconnections and reconnect safely; do not promise continuity of an ordinary dead SSH shell. |

### 6.6 Network progression

| ID | Priority | Requirement |
|---|---|---|
| NET-01 | P0 | Manual connections on reachable Wi-Fi or wired LAN using SSH/secure tunnel. |
| NET-02 | P1 | Optional LAN discovery and secure agent pairing. |
| NET-03 | P2 | Use existing VPNs (e.g., Tailscale/WireGuard) to connect across networks securely. |
| NET-04 | P2 | Persistent device identity independent of IP or VPN endpoint. |

## 7. UX acceptance scenarios

**A — Mixed Android connectivity:** Android A plugged into Windows, Android B plugged into Fedora, and Android C paired over Wi-Fi. All appear together with accurate host/route labels, and actions on B cannot affect A or C. Wireless scenario is P1; A+B is P0.

**B — Remote workspace:** open Windows terminal, WSL and Fedora SSH in separate tabs, browse Fedora over SFTP, and switch without leaving app (P0); later restore this layout (P1).

**C — Remote desktop:** select Fedora and open a configured desktop session with keyboard/mouse in a Senvakara pane (P1, gated by backend compatibility).

**D — Cross-host build:** build on one authorized host, send APK to an Android attached to another, install and view logcat, with status per step (P1).

**E — Failure:** pull Fedora USB cable / disable Wi-Fi; affected device transitions offline, unrelated sessions continue, and user sees safe recovery steps.

## 8. Non-functional requirements

- **NFR-01 Security:** verified host identity, encrypted transport, least privilege, no plaintext secrets or raw ADB exposure by default.
- **NFR-02 Reliability:** one failed adapter does not crash others; cancel and cleanup subprocesses/streams.
- **NFR-03 Privacy:** no mandatory cloud account, telemetry opt-in if ever introduced.
- **NFR-04 Performance:** target, not guarantee: desktop idle RAM <150 MB, Linux host agent idle <30 MB, startup <2 s and near-zero idle CPU under defined benchmark conditions; revise after actual measurements.
- **NFR-05 Compatibility:** Windows desktop client and generic Linux remote hosts first (Fedora is the reference test host; other distributions marked unverified until tested); future client OS support only after validation.
- **NFR-06 Accessibility:** keyboard navigation, legible status labels, color-independent indicators.
- **NFR-07 Auditability:** explicit errors, logs with redaction, reproducible tests and documented dependency licenses.

## 9. MVP definition & release criteria

**P0 scope only:** local/WSL/SSH terminal, SFTP, saved hosts, global device registry, Windows/Fedora USB Android discovery, basic routed ADB actions, secure connections. Embedded RDP/VNC and scrcpy video are **not** MVP blockers.

MVP passes when: (1) all P0 reference scenarios pass on the target hardware; (2) Android A and B are listed simultaneously and ADB command routing is isolated; (3) SSH host key is checked; (4) no plaintext credentials or exposed ADB port by default; (5) UI reports unavailable routes honestly; (6) benchmark recorded.

## 10. Out of scope for initial releases

Mandatory cloud backend, multi-tenant RBAC, building an IDE, universal access without Android debugging permission, own VPN implementation, public remote relay, custom SSH/RDP/ADB protocols, collaborative team features.

## 11. Product success measures

Baseline during PoC and track: steps to connect to remote host; time to issue an ADB action to Fedora-attached phone; successful cross-host actions; number of external app switches per user scenario; reconnection success by protocol; total RAM/CPU at idle and with sessions. Numeric goals are not yet validated.

## 12. Dependencies and open questions

Embedded scrcpy video and embedded RDP/VNC compatibility, Fedora Wayland behavior, adb transport routing, protocol choice for agent, identity matching across USB/Wi-Fi, license terms, credentials/keychain solution and native binary packaging. See [DECISIONS.md](DECISIONS.md) and [TECH_STACK.md](TECH_STACK.md).
