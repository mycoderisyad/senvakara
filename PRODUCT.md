# Product

<!-- impeccable:product-schema 1 -->

## Stack

Native Windows desktop application. Rust workspace; UI in Slint (pinned 1.18.1; see `docs/DECISIONS.md` ADR-023). There is no web view, so HTML/CSS tooling does not apply.

## Users

Three audiences, weighted equally (confirmed by the owner, 2026-11-21):

- **Developer** working on Windows with WSL and one or more Linux hosts over SSH, who wants terminals, files and remote desktop without switching apps.
- **Homelab user / sysadmin / devops** who manages several machines: sessions, files, health metrics, and VPN paths to the same host.
- **Android / Flutter developer** with physical phones attached to different hosts (USB on Windows, USB on a Linux host, Wi-Fi debugging), who deploys, mirrors and reads logcat.

They use the app at a desk, for long sessions, usually with several sessions open at once.

## Product Purpose

Senvakara is a local-first control center for computers, development environments and Android devices. From one window the user operates SSH, SFTP, local and remote terminals, remote desktop (VNC over SSH), Android ADB and screen mirroring, host metrics, saved workspaces and automation recipes. Success means the user no longer switches between a terminal app, an SFTP client, scrcpy, a VNC viewer and `adb` on several machines.

## Positioning

Topology-aware orchestration: an Android phone plugged into a Linux host appears as a first-class device in the app running on Windows, next to phones plugged into Windows or paired over Wi-Fi. Every device action is bound to a concrete host and transport. The product reuses SSH, ADB, scrcpy and VNC instead of reinventing them.

## Operating Context

- Reference setup: Windows 11 desktop, WSL, a Linux host (Fedora) on the LAN or over a user-managed VPN, and physical Android phones.
- Sessions are long-lived and run side by side: terminals, file transfers, mirrors, logcat, remote desktop.
- No account, cloud service or relay is required.

## Capabilities and Constraints

- Hosts: add, trust host key, connect, reconnect, VPN paths, capability detection.
- Terminals: PowerShell, WSL, SSH, tmux reattach.
- Files: SFTP, Android push/pull, direct host-to-device transfers.
- Android: device registry across hosts, wireless pairing, shell, apps, logcat, screenshot, mirroring with input.
- Remote desktop: X11 through on-demand x11vnc over SSH. Wayland is not supported.
- Workspaces, Flutter build-and-deploy, automation recipes.
- Constraints: Windows only today. Device identity is never merged by name or IP. No ADB or desktop port is exposed beyond loopback. Heavy streams start only when the user opens them. `adb.exe` is not bundled.
- Terminology: a *host* is a computer; a *device* is an Android phone; a *route* is how a device is reached (host plus USB or Wi-Fi).

## Brand Commitments

- Name: Senvakara. Vision line: "One Application. Every Device. Unified Control."
- Interface direction pinned by the owner (2026-11-21): in the manner of Termius but more minimal; dark theme only; no emoji anywhere in the interface; clean, light, professional, easy to understand.
- Native Windows title bar.

## Evidence on Hand

- Working v0.1.0 (Tauri, in git history) and 0.2.0 (native Slint) with measured benchmarks in `docs/TESTING.md`.
- Screenshots of the current UI and of the Termius reference were supplied in conversation; none are stored in the repository.
- No testimonials, customers or pricing exist. Do not invent them.

## Product Principles

1. Device-centric and host-aware: always show which host and route an action will use.
2. Local-first and secure by default: explicit trust, no silent fallbacks to another device.
3. On demand: nothing heavy runs until the user opens it, and it stops when they close it.
4. Honest state: discovered, trusted, connected, unauthorized and offline are different and look different.
5. One place: sessions live side by side and survive navigation.

## Accessibility & Inclusion

Keyboard navigation across the whole interface, visible focus, 4.5:1 contrast for normal text, and status never conveyed by color alone (`docs/DESIGN.md` section 8).
