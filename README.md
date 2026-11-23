# Senvakara

> **One Application. Every Device. Unified Control.**

Local-first control center for Windows: local shells and WSL, remote Linux hosts (SSH, SFTP, VNC), and Android devices on any managed host. Native Slint UI, Rust core, Windows only for now.

Full documentation starts at [docs/README.md](docs/README.md).

## Repository layout

```
apps/senvakara/        desktop app (Slint UI): src/ Rust controllers, ui/ .slint views, resources/ notices
crates/
  app/                 headless service layer used by the UI (hosts, terminals, files, Android, mirror, workflows)
  core/                shared ids, targets and error types
  storage/             SQLite registry + migrations/
  secrets/             credential store and log redaction
  metrics/             Linux host metrics read over SSH (/proc, df)
  adapters-ssh/        SSH, SFTP, remote PTY, loopback port forwarding
  adapters-adb/        ADB protocol, sync, wireless pairing
  adapters-pty/        local ConPTY / WSL
  adapters-scrcpy/     scrcpy server (embedded) and video/control streams
tools/
  bench/               whole-process benchmark (Node)
  licenses/            THIRD_PARTY_NOTICES generator (Node)
  test-fixtures/       code-less test APK
docs/                  PRD, architecture, decisions, tasks, testing, changelog
```

Each adapter crate has `examples/` with small probes against real hardware.

## Quick start

```powershell
cargo run -p senvakara
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Requires Rust 1.100 (MSVC). Android features need the user's own `adb`. Release steps are in [docs/README.md](docs/README.md#building-a-release-windows).
