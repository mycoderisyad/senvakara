# Roadmap — Senvakara

**Status:** milestone ordering, not promised dates. `TASKS.md` holds executable checklist; `PRD.md` defines requirements.

| Milestone | Scope | Exit gate |
|---|---|---|
| **M0 — Technical PoC** | Windows/Fedora SSH, SFTP, Windows+WSL PTY, safe remote ADB from Fedora-attached Android | Real hardware tests show host routing works; risks logged |
| **M1 — Desktop Foundation** | Tauri UI, registry, secure credentials, terminal tabs, SFTP | Windows/WSL/Fedora workflows inside Senvakara; stable persistence |
| **M2 — Multi-host Android MVP** | Windows USB + Fedora USB devices in one registry; correctly routed ADB actions | Two simultaneously connected phones independently controlled and accurately reported |
| **M3 — Android Control** | Wireless ADB onboarding, scrcpy streaming/control, APK install, logcat, file tools | Three mixed-route phones can be accessed; correct target and permission behavior |
| **M4 — Desktop + Monitoring** | RDP/VNC adapter, Fedora GUI, resource metrics | Configured remote desktop available; metrics truthful; tested on Wayland target |
| **M5 — Workspaces + Recovery** | Saved layouts, split sessions, transfer orchestration, reconnect | Restore layout and safely reconnect eligible sessions |
| **M6 — Developer Orchestration** | Flutter build and cross-host APK deploy, logs, progress | Build on one host, deploy to phone attached to another, verify result |
| **M7 — Automation** | Reusable recipes, history, improved recovery | Repeatable, cancelable, authorized workflow execution |
| **M8 — Secure Remote Networks** | VPN route integration, additional host networks | End-to-end remote connection over VPN with no public ADB exposure |

## Release gates

**Technical prototype:** M0. No promises about performance or integrated UI yet.

**Early Alpha / MVP:** M1–M2. Key differentiator demonstrated: device-centric Android over multiple hosts **plus** SSH/SFTP/local shell.

**Public Alpha:** M3–M4, subject to video backend feasibility; temporary native viewer fallback must be marked if embedded streaming not ready.

**Beta:** M5–M6; robust workspace/deployment experience and measured performance.

**Stable 1.0:** agreed P1 requirements met, security review, compatibility matrix, crash/error handling and third-party distribution audit. M7/M8 may ship after stable 1.0; no hard calendar commitment.

## Dependencies and sequencing

- Multi-host Android command routing **before** mirroring integration.
- RDP/VNC backend PoC and Wayland compatibility **before** committing to embedded UI implementation.
- Host/device registry and trust **before** workflows and automation.
- Transfer integrity and target identity checks **before** cross-host deployment.
- VPN **after** internal routing abstractions are stable, but avoid LAN assumptions from day one.

## Scope control

Do not include public relay servers, mandatory login, cloud database, team RBAC, own VPN implementation, custom SSH/RDP/ADB protocols or mobile Senvakara clients in MVP.

## Risks / milestone adjustments

If embedded streaming uses too much RAM/CPU or a backend cannot be bundled legally/technically, record the results in `DECISIONS.md`, keep an explicit fallback, and revise milestone/requirements transparently. No fabricated benchmarks or unsupported claims.
