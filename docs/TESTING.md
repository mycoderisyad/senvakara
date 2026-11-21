# Testing Strategy — Senvakara

**Status:** planning. No tests have been executed in this documentation-only package.

## 1. Goals

Verify correctness of device routing across Windows/Fedora, trust/security defaults, failure recovery, actual resource use, and integrated session behavior. Distinguish simulated tests from real hardware end-to-end tests.

## 2. Test layers

- **Unit (Rust/TS):** routing selection, status reducers, path validation, credential redaction, state machines, request parsing.
- **Component:** terminals, device cards, settings, actions, progress/cancel/error UI and keyboard navigation.
- **Integration:** SSH/SFTP/agent/ADB adapters with controlled test endpoints; migrations and persistence.
- **Hardware E2E:** actual Windows PC, Fedora PC, at least two Android phones USB-attached on different hosts; third Wireless ADB phone for P1.
- **Security:** unauthorized/expired credentials, forged discovery, wrong-device targeting, exposed ports, malformed messages.
- **Performance:** cold/warm startup, idle CPU/RAM, device monitoring overhead, 1/3 simultaneous video streams, remote terminal latency, transfer throughput.

## 3. Reference hardware matrix

| Setup | Requirement |
|---|---|
| Windows desktop + PowerShell/WSL | MVP P0 |
| Fedora remote on same Wi-Fi/Ethernet LAN | MVP P0 |
| Android A plugged USB into Windows | MVP P0 |
| Android B plugged USB into Fedora | MVP P0 |
| Android C paired via wireless debugging | P1 |
| Fedora running Wayland desktop service | P1 desktop PoC |
| Two identical Android models | P1 identity negative test |
| VPN on a separate network | P2 |

Record actual OS versions, Android APIs, ADB and scrcpy versions, CPU/RAM/GPU and network when testing. Do not assume Android Wireless Debugging works on all versions.

## 4. Critical scenarios

| Test ID | Scenario | Pass criteria |
|---|---|---|
| E2E-01 | Windows local and WSL shell | Distinct interactive tabs, correct input/exit state |
| E2E-02 | SSH Fedora | Verified host identity; interactive PTY; clear failure on bad key |
| E2E-03 | SFTP Fedora | Upload/download content matches; cancellation handled |
| E2E-04 | Android A USB Windows + B USB Fedora | Both shown at same time with correct host and route |
| E2E-05 | ADB command target B | Only B changes/responds; A untouched |
| E2E-06 | Disconnect Fedora USB B | B unavailable; A remains available; no misrouting |
| E2E-07 | Unauthorized Android | Accurate state; no bypass or silent failed commands |
| E2E-08 | Wireless Android C | Pair explicitly, route distinguishable, controlled with authorization |
| E2E-09 | scrcpy multiple devices | Independent streams/input; no cross-device events |
| E2E-10 | Fedora desktop | Compatible RDP/VNC session and input; disconnect isolated |
| E2E-11 | Cross-host Flutter build→install | Correct build host and correct Android target; per-step status |
| E2E-12 | Reopen workspace | Layout restores; nonrecoverable sessions shown correctly |

## 5. Failure / edge cases

- Firewall/client isolation blocks local peers.
- SSH host key changes; reject and require explicit re-trust.
- Agent or ADB version incompatible; show actionable error.
- Two devices with identical model/name; no accidental merge.
- USB unplug/replug changes ADB transport ID.
- Device switches between USB and Wi-Fi.
- Host link lost mid-transfer or APK install; no false success/unsafe replay.
- Flutter SDK absent/misconfigured; explain missing prerequisite.
- RDP desktop unavailable or Wayland backend unsupported; graceful degradation.
- Process child crash / cancel / app close; no orphaned heavy processes.
- Disk full, large directory, huge logcat stream, slow file transfer.

## 6. Performance budgets (aspirational)

| Metric | Planning target | Measurement |
|---|---|---|
| Desktop idle RAM | <150 MB | sum app process tree after 5 min idle, no active sessions |
| Linux host agent idle RAM | <30 MB | daemon RSS under no active work |
| Idle CPU | Near 0% | 5 min average with polling configured |
| Dashboard startup | <2 seconds | cold and warm startup separately |
| Monitoring | default 5 seconds | sampling overhead and network use |
| Terminal reconnect | <3 seconds when host reachable | from transient loss; distinguish new session from reattach |

Numbers are **targets requiring revision after measured baseline**, not an SLA. Document hardware/OS/build, multiple runs, p50/p95, total memory including sidecars, and screen session/resource scaling.

## 7. Security validation

- No publicly exposed ADB port, unauthenticated agent or unsecured remote desktop listener by default.
- Private keys/passwords not in logs, SQLite or crash reports.
- Permission denial cannot be bypassed with alternate route.
- Device selector bound to host and verified route, including on reconnect.
- Path traversal, command injection, malformed input and out-of-bounds payload rejected.
- Dependency audit and OSS license/redistribution checklist.

## 8. CI and manual gates

CI candidate: Rust fmt/clippy/tests, TS typecheck/lint/tests, migration test, dependency checks. Separate hardware test pipeline/manual checklist; absence of phone hardware is a **not tested** result, not pass.

MVP ships only after `E2E-01` through `E2E-07` pass on reference setup plus security basics. See `ROADMAP.md`.

## 9. PoC findings log

| Date | Task | Environment | Result | Notes |
|---|---|---|---|---|
| 2026-11-21 | POC-03 | Windows 11 Pro 10.0.26300, Rust 1.94.1 (MSVC), `portable-pty` 0.9.0, Windows PowerShell 5.1.26100, WSL 2.5.9 / Ubuntu-24.04 (kernel 6.6.87.2) | PASS | ConPTY emits a cursor-position query (`ESC[6n`) at startup and blocks until answered; xterm.js answers it, headless/test harnesses must reply `ESC[row;colR`. |
