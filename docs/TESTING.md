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
| 2026-11-21 | POC-02 | Linux reference host: Fedora Linux 45 (Xfce), kernel 7.3.4-300.fc45.x86_64, OpenSSH server on LAN port 22; client russh 0.65.2 (`ring` backend) | PASS | Wrong pinned fingerprint rejected before auth; ed25519 key auth; interactive PTY with resize (`stty size` → `40 100`). `aws-lc-sys` (russh default) needs NASM on Windows, so `ring` is used. |
| 2026-11-21 | POC-04 | same, russh-sftp 3.0.1 | PASS | 4 MiB round-trip byte-identical (up 2.05 s, down 4.21 s over Wi-Fi LAN, unoptimised debug build, single run); listing works; abandoning a 64 MiB upload after 300 ms leaves the session usable. Throughput not yet benchmarked. |
| 2026-11-21 | POC-05 | Android A: Xiaomi 2201117PG, Android 13 / API 33, USB on Windows (platform-tools 35.0.2). Android B: Redmi 5A, USB on Fedora (`/usr/bin/adb`) | PASS | A and B listed simultaneously; Linux ADB server reached only via SSH local forward `127.0.0.1:<random>` → Fedora `127.0.0.1:5037`; Fedora `ss -ltn` shows 5037 on `127.0.0.1` only; Windows ADB server on `127.0.0.1:5037` only; `-s` scoped `getprop` returns correct model per host; B's serial absent from Windows server; unknown serial fails with "not found" (no fallback). |
| 2026-11-21 | WebCodecs spike (POC-06 prerequisite) | Tauri 2 dev build, WebView2 157.0.4341.60 (Chromium 157) on the Windows reference client | PASS | `VideoDecoder.isConfigSupported` true for H.264 Baseline/Main/High, H.265, VP9, AV1; H.264 encoder also available. Annex-B H.264 encode→decode round trip: 5 chunks → 5 frames. Checked via app Diagnostics view over the WebView2 DevTools port. Other Windows machines may differ (GPU/driver/OS media features); keep a software-decoder fallback. |
| 2026-11-21 | CORE-04 (E2E-01 local part) | same app build | PASS | PowerShell + WSL tabs interactive in-app, resize via xterm fit addon, closing tab kills shell (0 child processes remain). React StrictMode double-mount did not leak a session. |
| 2026-11-21 | E2E-02 (in app) | App dev build → Fedora 45 reference host | PASS | Host added through UI, fingerprint confirmed against `ssh-keyscan`, interactive SSH tab. Changed-key simulation (pinned value replaced in the app DB): connect refused, warning dialog shown, wrong fingerprint cannot be pinned. |
| 2026-11-21 | E2E-03 (in app) | same | PASS | Listing in Files view; transfers through the same IPC the UI uses: 8 MiB round trip byte-identical, conflict protection, overwrite, cancel of 256 MiB upload with remote partial removed. Native file-picker steps were not automated. |
| 2026-11-21 | M1 exit gate | same | PASS | E2E-01, E2E-02, E2E-03 pass on the reference setup. Not yet covered: passphrase-protected keys end-to-end, reconnect after network loss (REC-01, P1). |
| 2026-11-21 | ADB host protocol spike | Windows ADB server (platform-tools 35.0.2, protocol 41), Android A | PASS | `host:version`, `host:devices-l`, `host:track-devices-l`, `host:transport-id:<id>` + `shell:` all behave as used; unknown transport id → `FAIL no device with transport id`. Legacy `shell:` gives no exit status. |
| 2026-11-21 | E2E-04..07 (partial) | App dev build; Android A on Windows; Android B on Fedora **not visible to Fedora's adb** at test time (`lsusb` shows `18d1:4e11`, no ADB interface) | PARTIAL | A: listed with host+route, Info and confirmed Shell OK, transport change and local server restart handled, Linux host disconnect leaves A available. B-side checks (simultaneous listing in app, command isolation A vs B, physical unplug, unauthorized) **not yet run**. |
| 2026-11-21 | E2E-04, E2E-05 | App dev build; Android A USB on Windows, Android B (Redmi 5A) USB on Fedora | PASS | Both listed at once with correct host/route; command isolation proven with a marker file in `/data/local/tmp` (present on B only, removed afterwards); confirmation/route mismatch refused. E2E-06 (physical unplug) and E2E-07 (unauthorized) still need a person at the phones. |

### Reference environment (POC-01)

| Component | Observed |
|---|---|
| Windows client | Windows 11 Pro 10.0.26300, WebView2 157.0.4341.60, Rust 1.94.1 MSVC, Node 24.14.0, WSL 2.5.9 (Ubuntu-24.04) |
| Linux reference host | Fedora Linux 45, **Xfce desktop** (not GNOME → GNOME Remote Desktop does not apply; M4 must evaluate xrdp / x11vnc), x86_64, sshd on LAN :22 reachable from Windows |
| Android A | Xiaomi 2201117PG, Android 13 (API 33), USB → Windows |
| Android B | Redmi 5A, USB → Fedora |
| Network | Same LAN (Wi-Fi), manual IP; no firewall changes were required for SSH |

