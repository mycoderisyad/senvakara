# Implementation Tasks — Senvakara

**Status:** planning. All boxes start unchecked until code and test evidence exist. **P0** MVP, **P1** stable-1.0 target, **P2** future. Keep task IDs stable; record evidence before checking off.

## Phase 0 — Feasibility / prerequisites

- [x] **POC-01 [P0]** Document reference Windows/Fedora versions, Wi-Fi/Ethernet reachability, firewall and SSH prerequisites. *Evidence 2026-11-21: reference environment table in TESTING.md §9.*
- [x] **POC-02 [P0]** Prove Windows → Fedora SSH with strict host-key checking. *Evidence 2026-11-21: `cargo run -p senvakara-adapters-ssh --example poc_linux -- <host> <user> <pinned-fp>` → 14/14 PASS (see TESTING.md §9).*
- [x] **POC-03 [P0]** Prove Windows local PTY and WSL PTY. *Evidence 2026-11-21: `cargo run -p senvakara-adapters-pty --example poc03_pty -- Ubuntu-24.04` → PowerShell=PASS, WSL=PASS (interactive input, resize to 100x40 confirmed via `stty size`, kill terminates child).*
- [x] **POC-04 [P0]** Prove SFTP listing/upload/download to Fedora. *Evidence 2026-11-21: `cargo run -p senvakara-adapters-ssh --example poc_linux -- <host> <user> <pinned-fp>` → 14/14 PASS (see TESTING.md §9).*
- [x] **POC-05 [P0]** Prove separate ADB operations against Android on Windows USB and Fedora USB from Windows via secure route. *Evidence 2026-11-21: `cargo run -p senvakara-adapters-ssh --example poc_linux -- <host> <user> <pinned-fp>` → 14/14 PASS (see TESTING.md §9).*
- [ ] **POC-06 [P1]** Benchmark remote scrcpy command/media/control path via Fedora; assess embedded renderer options.
- [ ] **POC-07 [P1]** Validate Fedora Wayland desktop sharing with selected RDP/VNC backend.
- [ ] **POC-08 [P0]** Record security/licensing/build constraints and PoC findings in decisions.

## Phase 1 — App skeleton + MVP terminal and SFTP

- [x] **CORE-01 [P0]** Bootstrap Tauri 2, Rust core, React+TS, formatting/lint/test workflows. *Evidence 2026-11-21: `apps/desktop` (Tauri 2.12 + React 19 + TS) in Cargo workspace; local `npm run typecheck/lint/test/build` and `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --workspace` pass; CI workflow `.github/workflows/ci.yml` added but not yet run on GitHub.*
- [ ] **CORE-02 [P0]** SQLite migrations + host/connection registry.
- [ ] **CORE-03 [P0]** Host list, status, manual add/edit/remove, connection errors.
- [x] **CORE-04 [P0]** Local Windows/WSL PTY integration and tab lifecycle. *Evidence 2026-11-21: in the running app (`npm run tauri dev`), PowerShell 5.1 and WSL Ubuntu-24.04 tabs accept typed input via xterm.js, terminal auto-fits (`stty size` → `43 120`), closing a tab leaves 0 shell/conhost child processes (checked with Win32_Process).*
- [ ] **CORE-05 [P0]** SSH interactive PTY adapter with verified server identity.
- [ ] **CORE-06 [P0]** SFTP browse/upload/download, progress/error/cancel.
- [ ] **CORE-07 [P0]** Secure credential/key lookup via OS store.
- [ ] **CORE-08 [P0]** Basic logging, error model, unit/integration tests.

## Phase 2 — Multi-host Android registry + safe ADB

- [ ] **AND-IMPL-01 [P0]** Local Windows ADB discovery with explicit transport selection.
- [ ] **AND-IMPL-02 [P0]** Fedora authenticated bridge (agent or SSH) with least privilege, versioned contract if agent used.
- [ ] **AND-IMPL-03 [P0]** Report Fedora USB Android devices to global registry.
- [ ] **AND-IMPL-04 [P0]** Route ADB commands by host/device identifier; handle stale route.
- [ ] **AND-IMPL-05 [P0]** Show local and Fedora Android devices simultaneously and prove command isolation.
- [ ] **AND-IMPL-06 [P0]** Handle unplug, unauthorized debugging and lost host without wrong-device commands.

**MVP gate:** All P0 items above tested on reference setup; see `TESTING.md`.

## Phase 3 — Post-MVP Android experience

- [ ] **AND-IMPL-07 [P1]** Wireless ADB onboarding, pairing and re-connect UX.
- [ ] **AND-IMPL-08 [P1]** scrcpy local mirroring and input control with lifecycle and cancel.
- [ ] **AND-IMPL-09 [P1]** Fedora-attached Android mirroring/control through protected path.
- [ ] **AND-IMPL-10 [P1]** APK install/uninstall, ADB shell, logcat, screenshots, push/pull.
- [ ] **AND-IMPL-11 [P1]** Multiple simultaneous device sessions without command mix-ups.

## Phase 4 — Remote desktop and monitoring

- [ ] **DESK-IMPL-01 [P1]** RDP authentication and remote keyboard/mouse adapter PoC.
- [ ] **DESK-IMPL-02 [P1]** VNC adapter PoC and Fedora Wayland compatibility decision.
- [ ] **DESK-IMPL-03 [P1]** Embedded session pane or documented temporary external-viewer fallback.
- [ ] **MON-IMPL-01 [P1]** Unprivileged host metrics collector; adjustable sample interval.
- [ ] **MON-IMPL-02 [P1]** Connection heartbeat and stale-device status.

## Phase 5 — Workspace, transfer and recovery

- [ ] **WORK-IMPL-01 [P1]** Save and reopen tabs/layout/target references.
- [ ] **WORK-IMPL-02 [P1]** Split view and explicit saved workspace switching.
- [ ] **FILE-IMPL-01 [P1]** Cross-host transfer and Android file operations with progress.
- [ ] **REC-IMPL-01 [P1]** Reconnect policy with bounded retry, safe state reconciliation.
- [ ] **REC-IMPL-02 [P2]** Optional tmux persistent SSH workflow.

## Phase 6 — Developer workflow

- [ ] **FLOW-IMPL-01 [P1]** Configured Flutter build host and SDK validation.
- [ ] **FLOW-IMPL-02 [P1]** Build→secure transfer→targeted install→optional launch/logcat orchestration.
- [ ] **FLOW-IMPL-03 [P1]** Task progress, cancellation, failure stops and audit trail.
- [ ] **FLOW-IMPL-04 [P2]** Recipe editor and reusable automation steps.

## Phase 7 — Beyond local network

- [ ] **NET-IMPL-01 [P2]** Optional VPN endpoint setup guidance and route registration.
- [ ] **NET-IMPL-02 [P2]** Test cross-network SSH/agent/ADB security over VPN.
- [ ] **NET-IMPL-03 [P2]** Route fallback and identity reconciliation after network changes.

## Always-on quality tasks

- [ ] **QA-01** Set up test fixtures/mocks for adapters and error states.
- [ ] **QA-02** Automated security regressions (credential exposure, agent auth, ADB host targeting).
- [ ] **QA-03** Whole-process memory/CPU + startup benchmarks; track deltas.
- [ ] **QA-04** Audit third-party licenses before shipping binaries.
- [ ] **QA-05** Run accessibility and keyboard-navigation checks.

## Completion rule

For each checked task, add a brief proof (PR/test link or command outcome) to project tracking when repository exists. A demo on only a local phone does **not** close multi-host Android tasks.
