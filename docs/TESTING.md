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
| 2026-11-21 | E2E-06 | same | PASS | USB cable of Android B physically unplugged from Fedora: B flipped to "Not connected" (route `unavailable`, label kept), action on B refused `DEVICE_OFFLINE` without touching any other device, Android A stayed available and answered `getprop`. |
| 2026-11-21 | E2E-07 | same | PASS | After "Revoke USB debugging authorizations" and replug (USB debugging on, prompt not yet accepted): Fedora adb reports `unauthorized`; app shows "Unauthorized" with accept-prompt guidance; Shell and Info refused `ADB_UNAUTHORIZED`; user label kept. After approval B returned to Available (new transport id) without refresh and answered `getprop`. Note: in "Charging only" USB mode the phone exposes no ADB interface at all and correctly shows as Not connected. |
| 2026-11-21 | M2 exit gate | same | PASS | E2E-04, E2E-05, E2E-06, E2E-07 pass with Android A on Windows USB and Android B on Fedora USB. |
| 2026-11-21 | ADB sync spike | Android A | PASS | `sync:` LIST/STAT/SEND/RECV/QUIT as implemented; 300 KB round trip identical; RECV of a missing file returns `FAIL open failed`; `exec:screencap -p` returns a valid PNG. |
| 2026-11-21 | AND-IMPL-10 (M3 part 1) | App dev build; A on Windows USB, B on Fedora USB via SSH tunnel | PASS | Screenshot, logcat stream, fixture APK install/uninstall, push/pull with conflict protection on both routes. Screenshots/logs were only checked structurally and deleted afterwards (private phone content). |
| 2026-11-21 | Mirroring (AND-IMPL-08/09, POC-06) | App dev build; scrcpy-server v3.3.3 (official SHA-256); A on Windows USB, B on Fedora USB via SSH tunnel | PASS | Both mirrored at once. Input verified on B via `dumpsys window` focus (shade opened by button and by UI swipe, closed by Back). Bug found and fixed: UI input events sent as independent invokes could arrive out of order and break gestures → now queued, with MOVE coalescing. Resource sample with two mirrors: ≈571 MB working set (app + WebView), ≈29 % of one core. Not measured yet: glass-to-glass latency, long-session stability. |
| 2026-11-21 | E2E-08 (Wi-Fi route) | App dev build; Android A over Wireless debugging (same Wi-Fi as the Windows PC) | PASS | Connected from the app; route listed separately (`RAFGT · Wi-Fi`, own transport id); shell, info and mirroring (≈0.8 s start) over Wi-Fi; disconnect isolates only the Wi-Fi route. No third physical phone was used: the Wi-Fi route of A stands in for "Android C". Error paths: malformed code/address → `INVALID_INPUT`, nothing listening → `UNAUTHORIZED`/`HOST_OFFLINE`; pairing code absent from the log. |
| 2026-11-21 | E2E-09 | same | PASS | Independent streams for A and B at once; input injected on B only (verified via `dumpsys window`). |
| 2026-11-21 | Bug fix: stale availability after restart | same | FIXED | Persisted route states were shown as live after an app restart (B "Available" while its host was not connected). Startup now resets all routes to `unavailable` until a watcher observes them; unit test added. |
| 2026-11-21 | MON-IMPL-01 | App dev build; Windows PC + Fedora 45 | PASS | Overview metrics for both hosts; Fedora values equal `free -b` (used = total − available), `df -B1 /`, `nproc`. |
| 2026-11-21 | POC-07 (detection) | Fedora 45 reference host | PASS (see next rows) | Boot target was `multi-user.target` (text mode) → detected as `none`; after the user started the GUI: X11 session, Xfce, display `:0` via lightdm. `x11vnc` not yet installed, so sharing is untested. |
| 2026-11-21 | Build hygiene | Windows dev machine | NOTE | Debug build cache reached 16 GB and filled drive D:; dev profile now keeps line tables only for workspace crates and no debug info for dependencies (target ≈ 2 GB). `sysinfo` pinned to 0.38 because 0.39+ needs rustc 1.95 (toolchain 1.94.1). |
| 2026-11-21 | SECURITY FINDING (fixed) | x11vnc 0.9.17 on Fedora 45 | FIXED | First design used `-unixsock … -rfbport 0 -nopw`. On this build `-unixsock` needs a patched libvncserver (client connections failed) and `-rfbport 0` still opened `[::]:5900` **without a password** on all interfaces. Found during testing within minutes; both processes were killed and 5900 verified closed. Replaced by loopback-only random port + one-time password + post-start `ss` guard that aborts on any non-loopback listener. |
| 2026-11-21 | DESK-IMPL-02/03 | App dev build → Fedora 45 Xfce/X11 via lightdm (`XAUTHORITY=/run/lightdm/<user>/xauthority`, found from the user's own processes; `-auth guess` failed) | PASS | Connected in ≈1.3 s, 1366×768; right-click changed 22/256 sampled pixels (menu), Escape restored most (7 differ, likely live content); listeners only 127.0.0.1/[::1]; no password file left; after Disconnect 0 x11vnc processes. Fixed during test: closing the pane did not always stop x11vnc → `desktop_close` now stops it explicitly. |
| 2026-11-21 | MON-IMPL-02 / PRD scenario E (network part) | App dev build; Fedora Wi-Fi turned off ≈45 s by the user | PASS | Timeline (3 s recorder): 18:31:58 ssh up, B available → 18:34:01 log "heartbeat: host … stopped answering" → 18:34:02 ssh down, ADB watch off, B unavailable, A available throughout. After Wi-Fi returned: Connect → host watching, B available. Detection latency is bounded by the 15 s interval + 6 s probe timeout. |
| 2026-11-21 | M4 exit gate | same | PASS (X11 targets) | Remote desktop on the Fedora X11 session (E2E-10), truthful metrics (MON-IMPL-01), heartbeat (MON-IMPL-02). RDP adapter deferred (no RDP target); Wayland sessions reported unsupported. |
| 2026-11-21 | REC-IMPL-01/02 | App dev build; Fedora 45 | PASS | Connection loss simulated by killing only the app's own `sshd-session` on Fedora (identified by start time; the user's other sessions untouched). Automatic recovery in ≈17 s without user action; tmux-backed tab reattached with its shell state intact. Test tmux session removed afterwards. |
| 2026-11-21 | FILE-IMPL-01 | App dev build; Fedora 45 ↔ Android A (USB on Windows) | PASS | Cross-host streaming both directions with matching SHA-256; conflict and target-mismatch refusals; test files removed from phone and host. |
| 2026-11-21 | WORK-IMPL-01/02 | App dev build; Fedora + Android A/B | PASS | 2×2 workspace (tmux terminal, B screen via Fedora, Fedora files, A logcat) saved, closed and reopened with tmux state intact; host-disconnected case explained per panel and restored by "Connect host"; DB migrated v1→v2 in place. |
| 2026-11-21 | M5 exit gate (E2E-12) | same | PASS | Layout restore, reconnect of eligible sessions (tmux), honest explanation of unrecoverable ones, automatic host reconnect, cross-host transfers. |
| 2026-11-21 | E2E-11 (FLOW-IMPL-01..03) | App dev build; Flutter 3.47.9 on Windows; Redmi 5A (Android 16 / SDK 36, arm64) on Fedora USB | PASS | Build on Windows → install on the Fedora-attached phone → launch → app logcat: all six steps succeeded in 135 s. Bugs found and fixed: (1) `flutter.bat` spawned from the GUI process failed with 0xC0000142 → now started with null stdin and CREATE_NO_WINDOW; (2) cancel killed only cmd.exe → now kills the whole process tree. Gradle daemons stay alive by design (reused by later builds). Debug APK of the fixture is 150 MB, release 41 MB. |
| 2026-11-21 | M6 exit gate | same | PASS | Build on one host, deploy to a phone attached to another, verified running. Remaining gap: Linux build hosts. |
| 2026-11-21 | FLOW-IMPL-04 (M7) | App dev build; Redmi 5A on Fedora USB | PASS | Recipe editor driven through the UI. Permission refusal happens before the run is created (no device side effects). Full 6-step recipe 31 s; embedded Flutter deploy step 34 s (warm Gradle); cancel ≈1 s. Bugs found and fixed: run progress of the previous recipe stayed visible after switching recipes; steps that never ran showed as "pending" instead of "skipped"; runs of deleted recipes were not visible anywhere → "All recipe runs" table. Not covered: recipes targeting a Wi-Fi route, two recipes running at once. |
| 2026-11-21 | M7 exit gate | same | PASS | Repeatable, cancellable, explicitly authorized workflow execution with history. |
| 2026-11-21 | NET-IMPL-01..03 (M8) | App dev build; user's WireGuard (WireGuard for Windows from the official installer, split tunnel `10.8.0.0/24`); Fedora wg0; Windows on phone hotspot over mobile data | PASS | LAN→VPN fallback without user action (same device/route ids), VPN→LAN return, phone shell over VPN 715–840 ms, only SSH leaves the PC, ADB loopback-only on Fedora. Setup findings: WireGuard for Windows with `AllowedIPs = 0.0.0.0/0` enabled its kill-switch and blocked the home LAN; Fedora's custom firewalld zone for wg0 allowed only DNS; a phone hotspot that also shares the phone's Wi-Fi still reaches the home LAN (not a separate network). Fixed during test: a stale LAN path held fallback for 10 s → 5 s. Not covered: Tailscale, VPN-only hosts added from scratch while away from the LAN (key check works over any path, but the first trust was done on LAN). |
| 2026-11-21 | M8 exit gate | same | PASS | End-to-end remote control over the user's VPN with no public ADB exposure. |

### Reference environment (POC-01)

| Component | Observed |
|---|---|
| Windows client | Windows 11 Pro 10.0.26300, WebView2 157.0.4341.60, Rust 1.94.1 MSVC, Node 24.14.0, WSL 2.5.9 (Ubuntu-24.04) |
| Linux reference host | Fedora Linux 45, **Xfce desktop** (not GNOME → GNOME Remote Desktop does not apply; M4 must evaluate xrdp / x11vnc), x86_64, sshd on LAN :22 reachable from Windows |
| Android A | Xiaomi 2201117PG, Android 13 (API 33), USB → Windows |
| Android B | Redmi 5A, USB → Fedora |
| Network | Same LAN (Wi-Fi), manual IP; no firewall changes were required for SSH |

