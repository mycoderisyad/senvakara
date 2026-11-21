# Implementation Tasks — Senvakara

**Status:** planning. All boxes start unchecked until code and test evidence exist. **P0** MVP, **P1** stable-1.0 target, **P2** future. Keep task IDs stable; record evidence before checking off.

## Phase 0 — Feasibility / prerequisites

- [x] **POC-01 [P0]** Document reference Windows/Fedora versions, Wi-Fi/Ethernet reachability, firewall and SSH prerequisites. *Evidence 2026-11-21: reference environment table in TESTING.md §9.*
- [x] **POC-02 [P0]** Prove Windows → Fedora SSH with strict host-key checking. *Evidence 2026-11-21: `cargo run -p senvakara-adapters-ssh --example poc_linux -- <host> <user> <pinned-fp>` → 14/14 PASS (see TESTING.md §9).*
- [x] **POC-03 [P0]** Prove Windows local PTY and WSL PTY. *Evidence 2026-11-21: `cargo run -p senvakara-adapters-pty --example poc03_pty -- Ubuntu-24.04` → PowerShell=PASS, WSL=PASS (interactive input, resize to 100x40 confirmed via `stty size`, kill terminates child).*
- [x] **POC-04 [P0]** Prove SFTP listing/upload/download to Fedora. *Evidence 2026-11-21: `cargo run -p senvakara-adapters-ssh --example poc_linux -- <host> <user> <pinned-fp>` → 14/14 PASS (see TESTING.md §9).*
- [x] **POC-05 [P0]** Prove separate ADB operations against Android on Windows USB and Fedora USB from Windows via secure route. *Evidence 2026-11-21: `cargo run -p senvakara-adapters-ssh --example poc_linux -- <host> <user> <pinned-fp>` → 14/14 PASS (see TESTING.md §9).*
- [x] **POC-06 [P1]** Benchmark remote scrcpy command/media/control path via Fedora; assess embedded renderer options. *Evidence 2026-11-21: own scrcpy v3.3.3 client over the ADB host protocol; via Fedora tunnel B starts in ~2.9 s (incl. first server push), local A in ~0.8 s; static-screen A ≈ 320 kbps; app + WebView working set ≈ 571 MB and ≈ 29 % of one core with two mirrors open (dev build). Glass-to-glass latency not yet measured. Renderer: WebCodecs H.264 → canvas.*
- [x] **POC-07 [P1]** Validate Fedora Wayland desktop sharing with selected RDP/VNC backend. *Evidence 2026-11-21: reference host runs Xfce on X11 (not Wayland); validated with x11vnc 0.9.17 — see TESTING.md §9.*
- [ ] **POC-08 [P0]** Record security/licensing/build constraints and PoC findings in decisions.

## Phase 1 — App skeleton + MVP terminal and SFTP

- [x] **CORE-01 [P0]** Bootstrap Tauri 2, Rust core, React+TS, formatting/lint/test workflows. *Evidence 2026-11-21: `apps/desktop` (Tauri 2.12 + React 19 + TS) in Cargo workspace; local `npm run typecheck/lint/test/build` and `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --workspace` pass; CI workflow `.github/workflows/ci.yml` added but not yet run on GitHub.*
- [x] **CORE-02 [P0]** SQLite migrations + host/connection registry. *Evidence 2026-11-21: `crates/storage` migration v1 via `PRAGMA user_version` (hosts, host_endpoints, ssh_settings, devices, device_routes, sessions); 6 unit tests (idempotent migration, FK enforcement, cascade, trust reset on address change).*
- [x] **CORE-03 [P0]** Host list, status, manual add/edit/remove, connection errors. *Evidence 2026-11-21: Hosts view in running app — added Fedora host via form, edit kept identity/trust, status badges text+icon (This PC / Connected / Not connected / Host key not trusted / error), remove behind in-app confirmation.*
- [x] **CORE-04 [P0]** Local Windows/WSL PTY integration and tab lifecycle. *Evidence 2026-11-21: in the running app (`npm run tauri dev`), PowerShell 5.1 and WSL Ubuntu-24.04 tabs accept typed input via xterm.js, terminal auto-fits (`stty size` → `43 120`), closing a tab leaves 0 shell/conhost child processes (checked with Win32_Process).*
- [x] **CORE-05 [P0]** SSH interactive PTY adapter with verified server identity. *Evidence 2026-11-21: in app, first-connect dialog showed fingerprint matching independent `ssh-keyscan`; SSH tab interactive (`<user>@<host>`, `stty size` 43 120); simulated changed key → connect refused `UNAUTHENTICATED`, dialog shows 'host key has changed', trusting a different fingerprint refused `CONFLICT`.*
- [x] **CORE-06 [P0]** SFTP browse/upload/download, progress/error/cancel. *Evidence 2026-11-21: Files view lists the remote home directory; via app IPC: 8 MiB upload + download SHA-256 identical, existing destination → `CONFLICT` both directions, explicit overwrite works, 256 MiB upload cancelled → `CANCELLED` and no partial left on host. UI overwrite-confirm click-through not automated (native file dialog).*
- [x] **CORE-07 [P0]** Secure credential/key lookup via OS store. *Evidence 2026-11-21: `crates/secrets` stores key passphrases in Windows Credential Manager (keyring-core 1.1 + windows-native-keyring-store 1.1); DB keeps only an opaque `passphrase_ref`; round-trip test against real Credential Manager passes. Encrypted-key login not yet exercised end-to-end (test key has no passphrase).*
- [x] **CORE-08 [P0]** Basic logging, error model, unit/integration tests. *Evidence 2026-11-21: structured `CoreError` codes incl. new `INVALID_INPUT`; redacted rotating log at `%LOCALAPPDATA%/dev.senvakara.desktop/logs/senvakara.log`; redaction unit tests; grep of log after E2E found no key material or `password=`/`passphrase=` values.*

## Phase 2 — Multi-host Android registry + safe ADB

- [x] **AND-IMPL-01 [P0]** Local Windows ADB discovery with explicit transport selection. *Evidence 2026-11-21: `crates/adapters-adb` speaks the ADB host protocol directly (ADR-018); app watches the local server with `track-devices-l` (no polling), finds the user's `adb.exe` without bundling it, auto-restarts after `adb kill-server` (device went Not connected → Available again).*
- [x] **AND-IMPL-02 [P0]** Fedora authenticated bridge (agent or SSH) with least privilege, versioned contract if agent used. *Evidence 2026-11-21: on host Connect the app runs `adb start-server` on the Linux host and forwards `127.0.0.1:<random>` → host `127.0.0.1:5037` over the existing SSH connection; Devices view shows the host as watching; Fedora `ss -ltn` still shows 5037 on loopback only. No agent needed.*
- [x] **AND-IMPL-03 [P0]** Report Fedora USB Android devices to global registry. *Evidence 2026-11-21: after Connect, Android B (Redmi 5A, USB on Fedora) appears in the registry as `Fedora laptop · USB` with its own device id and transport id from the tunnelled server.*
- [x] **AND-IMPL-04 [P0]** Route ADB commands by host/device identifier; handle stale route. *Evidence 2026-11-21: every action re-reads the live device list and requires same host + serial + transport id (`ROUTE_STALE` otherwise); bogus transport id rejected by the server path; `adb reconnect` changed transport 1→2 and the card followed without creating a duplicate device; UI shell on Android A showed the full target in a confirmation and returned A's model only.*
- [x] **AND-IMPL-05 [P0]** Show local and Fedora Android devices simultaneously and prove command isolation. *Evidence 2026-11-21: A (`RAFGT · USB`) and B (`Fedora laptop · USB`) listed together; marker file written via B's route exists on B and not on A, then removed; `getprop` per route returns each phone's own model; request whose confirmed serial differs from the route's device refused with `ROUTE_STALE`.*
- [x] **AND-IMPL-06 [P0]** Handle unplug, unauthorized debugging and lost host without wrong-device commands. *Evidence 2026-11-21: physical unplug of Android B → route `unavailable`, action refused `DEVICE_OFFLINE`, A unaffected; revoked USB debugging → B shown "Unauthorized" with guidance, Shell/Info refused `ADB_UNAUTHORIZED`, then after approval B returned to Available on its own; Linux host disconnect and local `adb kill-server` handled (see TESTING.md §9).*

**MVP gate:** All P0 items above tested on reference setup; see `TESTING.md`.

## Phase 3 — Post-MVP Android experience

- [x] **AND-IMPL-07 [P1]** Wireless ADB onboarding, pairing and re-connect UX. *Evidence 2026-11-21: "+ Add over Wi-Fi" dialog (mDNS scan via `host:mdns:services`, `host:pair:<code>:<addr>`, `host:connect`), pairing through any watched host's ADB server; user connected Android A (Android 13) over Wi-Fi from the app; shown as a separate `RAFGT · Wi-Fi` route next to its USB route (no auto-merge); Disconnect → route unavailable and actions refused `DEVICE_OFFLINE` while USB route keeps working; reconnect restores it. Pairing code validated (6 digits), never stored or logged. Redmi 5A (Android < 11) cannot use Wireless debugging pairing.*
- [x] **AND-IMPL-08 [P1]** scrcpy local mirroring and input control with lifecycle and cancel. *Evidence 2026-11-21: Device → Screen → Start mirroring on Android A: live H.264 (576×1280, ~29 fps when the screen changes); session stops with the pane, on stream end, or when the route disappears.*
- [x] **AND-IMPL-09 [P1]** Fedora-attached Android mirroring/control through protected path. *Evidence 2026-11-21: Android B mirrored through the Fedora SSH tunnel (720×1280); notification-shade button, swipe-down from the canvas, and Back verified on the device via `dumpsys window` focus changes (launcher → NotificationShade → launcher). Concurrent with A's session.*
- [x] **AND-IMPL-10 [P1]** APK install/uninstall, ADB shell, logcat, screenshots, push/pull. *Evidence 2026-11-21: device detail view (Screen/Logcat/Apps/Files). On Android A (Windows USB) and Android B (via Fedora tunnel): `screencap` PNG via `exec:`, saved to disk; logcat streamed in UI (730 lines in 4 s, stop ends stream); code-less fixture APK (`tools/test-fixtures`) installed via sync push + `pm install -r`, listed, uninstalled, no temp file left; 1 MiB push/pull SHA-256 identical, existing file → `CONFLICT`, `..` paths → `INVALID_INPUT`. Interactive `adb shell` PTY is still the one-shot Shell dialog.*
- [x] **AND-IMPL-11 [P1]** Multiple simultaneous device sessions without command mix-ups. *Evidence 2026-11-21: concurrent sessions without mix-ups — mirrors of A (Windows USB) and B (Fedora tunnel) at once with input on B only; A via USB and via Wi-Fi as two routes, each action landing on the chosen route; shells/logcat per route bound to transport ids.*

## Phase 4 — Remote desktop and monitoring

- [ ] **DESK-IMPL-01 [P1]** RDP authentication and remote keyboard/mouse adapter PoC. *Deferred: no RDP target in the reference setup (Linux host uses X11/VNC); needed for Windows or xrdp hosts.*
- [x] **DESK-IMPL-02 [P1]** VNC adapter PoC and Fedora Wayland compatibility decision. *Evidence 2026-11-21: Fedora 45 Xfce (X11, lightdm) shared in-app: connect ≈1.3 s, 1366×768, right-click/Escape reach the host; listeners only on 127.0.0.1/[::1]; password file removed; 0 x11vnc processes after closing. Wayland: detected and reported unsupported (decision: X11 only for now). Text-mode host: terminal only.*
- [x] **DESK-IMPL-03 [P1]** Embedded session pane or documented temporary external-viewer fallback. *Evidence 2026-11-21: embedded noVNC pane (Hosts → Desktop), no external viewer needed for X11 hosts.*
- [x] **MON-IMPL-01 [P1]** Unprivileged host metrics collector; adjustable sample interval. *Evidence 2026-11-21: Overview shows CPU %, memory, system disk, network rates, uptime (and load on Linux) for this PC (`sysinfo` 0.38) and Fedora (one read-only `/proc` + `df` command over SSH, no agent, no root); values matched `free`/`df`/`nproc` on the host; interval 2/5/10/30 s, sampling only while the page is open.*
- [x] **MON-IMPL-02 [P1]** Connection heartbeat and stale-device status. *Evidence 2026-11-21: Fedora Wi-Fi switched off by the user → at 18:34:01 the 15 s heartbeat logged "stopped answering", dropped the SSH connection and ADB tunnel; Android B became unavailable while Android A (USB and Wi-Fi routes) stayed available. After Wi-Fi returned, Connect restored the host and B became available again. Startup also resets stale route states. Reconnect is manual for now (automatic reconnect is REC-IMPL-01).*

## Phase 5 — Workspace, transfer and recovery

- [x] **WORK-IMPL-01 [P1]** Save and reopen tabs/layout/target references. *Evidence 2026-11-21: schema v2 (`workspaces`, `workspace_panels`, migrated in place on the real app DB 1→2); Workspaces view saves name + layout + panel targets (terminal incl. tmux name, screen, files, logcat, desktop); "Fedora + phones" 2×2 reopened with all four panels live and the tmux shell state (`WS_MARK=restored-7`) intact.*
- [x] **WORK-IMPL-02 [P1]** Split view and explicit saved workspace switching. *Evidence 2026-11-21: 1 / 2-column / 2×2 split layouts, switching between saved workspaces unmounts and stops the previous panels; with the Linux host disconnected the three dependent panels explained why (host not connected / device not connected with route) while the Windows logcat panel kept running; "Connect host" restored all four.*
- [x] **FILE-IMPL-01 [P1]** Cross-host transfer and Android file operations with progress. *Evidence 2026-11-21: Files → "Send to device…" and device Files → "Send to host…" stream SFTP ↔ ADB sync with no copy on this PC. Fedora → Android A (Windows USB): 3 MiB in 2.2 s, SHA-256 on the phone identical; Android A → Fedora: 1.5 s, SHA-256 identical, no `.part` left; existing target → `CONFLICT`; mismatched confirmed serial → `ROUTE_STALE`; size verified after each copy; partials removed on failure/cancel. Android file push/pull from this PC was already covered by AND-IMPL-10.*
- [x] **REC-IMPL-01 [P1]** Reconnect policy with bounded retry, safe state reconciliation. *Evidence 2026-11-21: hosts the user connected are reconnected after a heartbeat loss with bounded backoff (2,4,8,16,32,60 s… max 20 attempts), stopping at once on authentication failure and when the user disconnects/removes the host; nothing is replayed. Test: the app's own sshd session on Fedora was killed at 18:55:45 → ssh down 18:55:47 → reconnecting 18:55:56 → ssh up 18:55:59 → ADB watch + Android B available 18:56:02, Android A unaffected; UI shows "Reconnecting (attempt n)".*
- [x] **REC-IMPL-02 [P2]** Optional tmux persistent SSH workflow. *Evidence 2026-11-21: "+ tmux" SSH tab runs `tmux new-session -A -s senvakara-…`; after the connection was killed, ↻ reattached and `$SVK_MARK` set before the loss was still present. Plain SSH tabs reopen a new shell (no false promise of continuity).*

## Phase 6 — Developer workflow

- [x] **FLOW-IMPL-01 [P1]** Configured Flutter build host and SDK validation. *Evidence 2026-11-21: Deploy view detects the Flutter SDK on this PC even when not on PATH (found `%USERPROFILE%lutterinlutter.bat`, reports `Flutter 3.47.9`), validates the project (pubspec.yaml + android/app, applicationId read from build.gradle(.kts)). Linux hosts as build hosts are refused with a clear message (no Flutter SDK on the reference Linux host; not yet implemented).*
- [x] **FLOW-IMPL-02 [P1]** Build→secure transfer→targeted install→optional launch/logcat orchestration. *Evidence 2026-11-21: hello-world fixture (`flutter create`, kept in `target/flutter-fixture`; the user's own projects were not touched) built in release on Windows (92 s, 41 MB, SHA-256 recorded), installed on Redmi 5A plugged into the Fedora host through the SSH-tunnelled ADB route, launched (pid verified) and 15 s of its logcat shown — 135 s end to end (PRD scenario D). Test app uninstalled afterwards.*
- [x] **FLOW-IMPL-03 [P1]** Task progress, cancellation, failure stops and audit trail. *Evidence 2026-11-21: per-step status with live build log; a failure stops the run (first attempt failed at build with 0xC0000142 and every later step was recorded as skipped); Cancel during build stopped the whole flutter/cmd/dart process tree within ~0.8 s (0 processes left); runs and step outcomes kept in `workflow_runs`/`workflow_steps` (schema v3) and shown as History.*
- [ ] **FLOW-IMPL-04 [P2]** Recipe editor and reusable automation steps.

## Phase 7 — Beyond local network

- [ ] **NET-IMPL-01 [P2]** Optional VPN endpoint setup guidance and route registration.
- [ ] **NET-IMPL-02 [P2]** Test cross-network SSH/agent/ADB security over VPN.
- [ ] **NET-IMPL-03 [P2]** Route fallback and identity reconciliation after network changes.

## Always-on quality tasks

- [ ] **QA-01** Set up test fixtures/mocks for adapters and error states.
- [ ] **QA-02** Automated security regressions (credential exposure, agent auth, ADB host targeting).
- [ ] **QA-03** Whole-process memory/CPU + startup benchmarks; track deltas.
- [ ] **QA-04** Audit third-party licenses before shipping binaries. *Includes shipping the full Apache-2.0 text for the bundled scrcpy-server (see `apps/desktop/src-tauri/resources/THIRD_PARTY_NOTICES.md`).*
- [ ] **QA-05** Run accessibility and keyboard-navigation checks.

## Completion rule

For each checked task, add a brief proof (PR/test link or command outcome) to project tracking when repository exists. A demo on only a local phone does **not** close multi-host Android tasks.
