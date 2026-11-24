# Changelog — Senvakara

Changes to documentation and (later) shipped product are tracked separately. Format inspired by Keep a Changelog; no released binary is implied.

## [Unreleased]

## [0.2.2] - 2026-11-24

### Changed — toolbar, SSH quick connect, empty states, SFTP overhaul

- **Single-row toolbar & SSH quick connect**: `PageToolbar` is one row with the same order on every page: the page's actions on the left with the filled add button first (New host, Add over Wi-Fi, New key, New workspace, New recipe), then the compact search field, and Refresh at the far right. On Hosts, the search bar doubles as a Termius-style SSH quick-connect bar (`Find a host or ssh user@hostname...`) with an inline **Connect** pill that accepts `ssh user@host`, `ssh -p port user@host`, `ssh -l user host`, `user@host[:port]`, and IPv6 targets. Matching saved SSH hosts open straight into a terminal (after host-key verification if untrusted); new targets open the Quick Connect host form pre-filled with address, port, and username.
- **Empty states without a button**: the empty views of Hosts, Devices, Keychain and Workspaces no longer carry a button. The add button stays in the toolbar, in the same corner whether the list is empty or not, and the empty state names it in its sentence. The empty state is also centred in its frame now (its content used to sit 32 px low, down to the bottom border).
- **SFTP UI/UX overhaul**: Redesigned the dual-pane SFTP view for a cleaner edge-to-edge layout modeled on Termius — clickable host-switcher pill (replacing the separate **Change** button), toggleable compact inline **Filter**, quiet **Actions** dropdown, tinted folder breadcrumbs, full-bleed file table, and removal of the bottom **Copy to other side** footer bar.
- **SFTP file management, right-click menu & full Actions dropdown**: Added backend and local support for **Rename** (`F2`), **Delete** (`Delete`, recursive on folders without following symlinks, always gated by a confirmation dialog), **Edit Permissions** (`chmod` with Owner/Group/Public R/W/X checkboxes and live octal preview on SSH hosts), **Hide/Show Hidden Files** (`Ctrl+H`), **Select All** (`Ctrl+A`), and **Close** (disconnect side back to host picker), available from both the right-click context menu and the **Actions** dropdown.

### Fixed — 2026-11-24

- **The menu next to "+" opens what is picked**: a click on a shell or host in the tab strip's menu closed the menu and opened nothing, because the row closed the menu before it told the strip what was picked. The same order was in the `Select` drop-down (Settings, Automations), which is fixed too. Ctrl+Shift+1 to 9 were not affected.
- **A delete that fails says so**: when a file or folder could not be deleted in the SFTP pane, the message was cleared by the listing that follows the delete. It now stays over the new listing.

### Internal — 2026-11-24

- `files-view/pane.slint` (1,050 lines) is split by what each part draws: `header.slint` (place, filter, Actions menu), `path-row.slint` (Back, Forward, Up, path), `strips.slint` (new folder, rename, permissions); `pane.slint` only puts them together. `files/manage.rs` and `inventory/quick_connect.rs` lost their repeated blocks; renaming and deleting on this PC are plain functions with tests in `files/local.rs`.

### Documentation — 2026-11-21

- Established project name **Senvakara** (working name, availability checks pending).
- Split initial product brainstorming into 14 Markdown files.
- Defined device-centric multi-host architecture: Windows USB, Fedora USB and authorized Wi-Fi ADB in one registry.
- Documented SSH, SFTP, local/WSL terminal, RDP/VNC, scrcpy, monitoring, workspaces, cross-host Flutter deployment and automation.
- Added phased roadmap, MVP boundary, security requirements, proposed SQLite model, API contract, tests and AI coding agent rules.

### Documentation — 2026-11-21 (planning update)

- Generalized remote host target from Fedora to generic Linux; Fedora remains the reference test host (ADR-013).
- Added ADR-014..017: remote ADB via SSH port-forward, no bundled `adb.exe`, candidate library set, embedded streaming approach.

### Changed — 2026-11-22 (after 0.2.0)

- Idle CPU on Home: a health sample now redraws only that host's card instead of rebuilding every list, and this PC's disk is read for free space only. Measured 1.20 % of one core before, 0.86 % after (60 s, release build, Intel GPU).
- The default renderer is Slint's software renderer instead of FemtoVG/OpenGL: the Intel OpenGL driver kept a time-critical thread busy at about 0.5 % of a core with no frames drawn. Idle on Home is now 0.10–0.23 % of one core. `SLINT_BACKEND=winit-femtovg` brings OpenGL back. Settings shows the renderer in use.
- The window title bar is asked to be dark whatever the Windows theme (winit window theme). Seen dark on a PC whose Windows theme is already dark; not yet checked with the light theme.
- Internal, no behaviour change: the app's largest source files were split into smaller modules (`apps/senvakara/src` and `ui`), and `docs/AGENTS.md` now points at the Rust guideline skill in `.claude/skills/rust-guidelines`. Evidence under QA-06 in `docs/TASKS.md`.

### Added — 2026-11-22 (SFTP tab, drag and drop, folders, host to host; not yet released)

Built and tested against in-process fakes; not yet tried by hand in the running app or against a real host (see FILE-DND-01, FILE-DIR-01 and FILE-H2H-01 in `docs/TASKS.md`).

- "SFTP" on the New Tab page and in the palette opens a tab named **SFTP**: this PC on the left, and on the right "Connect to host" with a **Select host** button. The list it opens has a search field, the SSH hosts (subtitle "ssh, user"; connected ones say so), the phones that are available and a **Local** button. Choosing a host that is not connected connects it; a host whose key is not trusted yet shows the host-key dialog first and is listed once the key is accepted. Either side can show another place with **Change**; this replaces the drop-down of places.
- The file list has the columns Name, Date Modified, Size and Kind. A click on a header sorts by it, a second click turns the order around; folders always come first. On a host the type and permissions ("drwxr-xr-x") are shown under the name. A path bar shows the folders down to the current one, each one clickable, with Back and Forward; a click next to it (or Actions > Type a path) brings back the typed path. A **Filter** field keeps the names that contain the text. **Actions** has Refresh, New folder (this PC and hosts; a phone says why not), Copy to other side and Type a path.
- **Drag and drop**: rows can be dragged to the other list or onto a folder of either list; everything selected goes along, and dragging a row that is not selected selects it first. The drop runs the same checks and questions as Copy to other side (phone confirmation, "Replace existing file?"). Files and folders dropped from Windows Explorer are uploaded to the side that shows a host or a phone (the side the pointer was over last, else the other one); with no host or phone on either side a note says so.
- **Folders** are copied with everything in them between this PC and a host and between hosts: the folders are created first, then the files, as one job with progress over the whole tree. Cancel stops between files and removes the file in progress. Links are not followed and are counted in the result ("2 links or unsupported items skipped"). A folder that already exists asks first; "Replace" merges into it. Folders to or from a phone are still refused, with a message.
- **Host to host**: a file or folder is copied from one SSH host to another through this PC without a temporary copy on it, 256 KiB at a time, with progress and Cancel. The destination is written as `<name>.part` and renamed when the size matches; a failed or cancelled copy removes it. The same host on both sides works for a different folder; copying onto the same place, or a folder into itself, is refused.

### Added — 2026-11-22 (New Tab page and shell menu, not yet released)

Built and unit-tested; not yet tried by hand in the running app (see TAB-NEW-01 and TAB-NEW-02 in `docs/TASKS.md`).

- "+" and Ctrl+Shift+T now open a **New Tab** page instead of a PowerShell tab: a search field over recent connections, all hosts, the shells of this PC, phones and SFTP. Up, Down and Enter choose; Esc clears the search or closes the page. What is picked takes the page's place in the tab strip. Remote Desktop opens in its own window and leaves the page open with a note.
- A chevron next to "+" lists the local shells (Windows PowerShell, PowerShell 7, Command Prompt, Git Bash, WSL distributions; only the ones installed), then hosts, then phones. Ctrl+Shift+1 to 9 open the first nine entries.
- New local shells: PowerShell 7, Command Prompt and Git Bash. Shells are looked for in the background at startup and when Home is refreshed, no longer on the UI thread.

### Changed — 2026-11-23 (interface restyle and fixes, not yet released)

Built, unit-tested and the Home screen looked at in the running app; the other screens and the fixed flows still need a manual pass.

- New look, measured from Termius's dark theme: navy surfaces, 14 px Inter (bundled, OFL) instead of a font name Windows did not have, 36 px controls with 10 px corners, host cards with a filled colour tile, pill-shaped session tabs, a sidebar with the product name, and page headers with a state line. Hosts and Devices are now separate pages, and empty lists say what to do. The accent stays Senvakara's teal.
- Home has a Refresh button (hosts, phones and the local shell list).
- Window shortcuts (Ctrl+Shift+T, W, K, 1 to 9, Ctrl+Tab) keep working after switching to a Files, screen, logcat or remote desktop tab.
- Copying several files that already exist at the destination asks about each one in turn instead of silently skipping all but the last; a cancelled or failed copy refreshes the destination list; replacing a file on a host is now written to a temporary name first, so cancelling keeps the old file.
- A local shell that fails to start no longer jumps to Home; on the New Tab page the error appears there.
- Host form: a tag typed but not added is kept on Save; removing one host no longer closes the form of another; a VPN path may share the address on another port; changing only the letter case of a Remote Desktop address asks for the password again. Remote Desktop hosts no longer count towards "reachable" or appear under "ADB on each host", and a failed launch selects the host so the error shows.
- Phones: a confirmed deploy runs only for the phone that was confirmed; a hidden screen tab no longer converts or draws frames; deploys inside recipes no longer appear as stand-alone deploys; Wi-Fi scan results can be clicked to fill the field; searching no longer drops the selected phone; card actions on a phone respect an unsaved host form; restarting logcat no longer repeats lines.
- Recipes ask before unsaved edits are dropped; closing a Run tab right after starting cancels the run; closing a workspace with unsaved layout changes asks first.
- Phones on a Linux host now appear as soon as the host is opened in any way (a terminal, Files, a desktop, or Connect in the host form), not only after the separate Connect action. adb installed under `~/Android/Sdk` is found too; a host without adb says so under "ADB on each host", and a phone the host's ADB may not open (Linux udev permissions) is shown as "No permission" with what to do. Checked on the reference Fedora laptop: a Redmi 5A on its USB appeared after opening the host through SFTP.
- A question that comes up while another dialog is open now waits for it instead of replacing it.
- A screen tab that is not on screen no longer decodes video at all; when it comes back the phone is asked for a fresh picture (scrcpy "reset video", control type 17, checked against the scrcpy v3.3.3 source). Not yet tried with a phone.
- Remote Desktop hosts open with the user name filled in, also without a saved password: Senvakara writes a small `.rdp` file (address and user only, never a password) under its data folder and opens it with `mstsc.exe`. Not yet tried against a Windows target.
- An encrypted private key now asks for its passphrase before Save, in the Keychain and for a key file in the host form.
- Drop-down lists use the app's own style; session tabs that do not fit scroll sideways and the active tab is kept in view.
- SFTP is a pinned tab next to Home, as in Termius (also "Open SFTP" in the command palette); it left the New Tab page. Every page now has the same toolbar: search on top, then the page's own actions on the left in one button style (New host, Add over Wi-Fi, New key, New workspace, New recipe), Refresh on the right. Automations has less text: one short line per card, the Flutter path only when something is wrong, content kept to a readable width.

### Added — 2026-11-22 (host credentials and Keychain, not yet released)

Built and unit-tested; not yet tried by hand in the running app (see HOST-UI-02, CRED-02 and DESK-IMPL-04 in `docs/TASKS.md`).

- New host form in the inspector, laid out as cards: Address, General (label, parent group, tags), "SSH on [port] port", Credentials and Startup snippet. A host can sign in with a password, a key from the Keychain (with its certificate), a key file as before, or a security key through the Windows OpenSSH agent. A saved password is shown only as "Password saved", with Replace and Remove. Hosts that sign in with a key file keep working and open in the same form.
- **Connect** in the form saves the host and opens it: for a new SSH host the host-key question comes first, then the terminal.
- Remote Desktop hosts: choose "Remote Desktop" while adding a host. Its card opens it in Windows' own Remote Desktop client (`mstsc.exe`); the password is kept in Windows Credential Manager. Changing the address or user name of such a host asks for the password again.
- Hosts on Home are grouped by parent group (hosts without one first), and search also finds a host by its user name, group or tags.
- Keychain section: add an SSH key by pasting it, importing a key file or dragging the file from Explorer onto the window; add a key with its certificate or a FIDO2 security key by its public key; edit and delete keys. A key that a host still signs in with cannot be deleted. Private keys and passphrases go to Windows Credential Manager and are never shown again.
- The startup snippet of a host is stored as plain text with the host, not as a secret.

### Changed — 2026-11-23 (toolchain and Slint upgrade, not yet released)

- Built with Rust 1.100.0 (was 1.94.1) and Slint 1.19.0 (was 1.18.1). CI and the new release workflow pin the same Rust version, so Clippy in CI matches local builds. Checked: fmt, Clippy with `-D warnings` and `cargo test --workspace` pass; the app starts and Home draws with the dark title bar. Not yet checked by hand: terminal, SFTP, mirror and remote desktop on Slint 1.19.
- Release workflow (`.github/workflows/release.yml`): a `vX.Y.Z` tag builds the NSIS installer and a portable zip on GitHub and publishes them as a GitHub Release; it can also be run by hand for a build artifact only.
- Third-party notices regenerated for the new dependency versions.

## [0.2.0] - 2026-11-22 (built, not yet signed off)

Installer `senvakara_0.2.0_x64-setup.exe` (NSIS, per-user) is built but has not been run; install, upgrade from 0.1.0 and uninstall are untested.

### Changed

- The desktop app is now a native Slint application (`apps/senvakara`, ADR-023). The Tauri + React app of 0.1.0 was removed; it remains in git history.
- Navigation: an inventory Home (Hosts, Devices, Workspaces, Automations, Settings) with an inspector, and every session as a tab: terminals (PowerShell, WSL, SSH, tmux), Android screen, logcat, dual-pane files (PC, host, phone), remote desktop, workspaces, automation runs. Command palette on Ctrl+Shift+K.
- Backend moved into the UI-independent crate `crates/app`. Only one Senvakara instance per data folder can run.
- Terminal emulation (`alacritty_terminal`), H.264 decoding (`openh264`, ADR-024) and the VNC client (`vnc-rs` on the SSH channel) are native; WebView2 and Node.js are no longer needed.
- Idle memory about 51 MB private (0.1.0: 204 MB); startup 0.14–0.50 s.

### Known gaps

- Many flows were exercised only up to their confirmation, or not on hardware (a phone behind the Linux host, two mirrors, Flutter deploy, recipe steps other than wait). See TESTING §9 and TASKS Phase 8.
- Idle CPU on Home is higher than 0.1.0 (0.73 % of one core against 0.08 %).
- Folder copy in Files, a forced-dark title bar, Tight/JPEG rectangles in remote desktop and the WebCodecs diagnostics page (replaced by a decoder self-test) are not there.

## [0.1.0] - 2026-11-21

First Windows installer (`Senvakara_0.1.0_x64-setup.exe`, NSIS, per-user, no admin rights; not code-signed, so Windows SmartScreen may warn). Third-party notices ship in the install folder.

### Added

- Desktop shell (Tauri 2 + React + TypeScript) with sidebar navigation; unbuilt sections state their planned milestone.
- Local terminal tabs: Windows PowerShell and each installed WSL distribution (ConPTY), resize, close kills the process.
- Diagnostics view: WebCodecs codec support and H.264 round-trip check for the local WebView2.
- Hosts: add/edit/remove Linux hosts, first-connect host-key verification dialog, changed-key warning, connect/disconnect, detected OS/arch/ADB shown per host.
- SSH terminal tabs alongside local tabs.
- Files: browse remote folders over SFTP, upload/download with progress, cancel, and confirmation before replacing files.
- Key passphrases stored in Windows Credential Manager; local SQLite stores only metadata.
- Redacted application log in the user's local app-data folder.
- Devices: Android phones from this PC and from connected Linux hosts in one list, labelled with host and route; Info and confirmed one-shot Shell bound to the exact device; rename/forget; live updates without polling.
- Device page: screenshot (save as PNG), streaming logcat with filter, user-app list with APK install/uninstall, device file browser with push/pull — every mutating action confirms the exact device and route.
- Live screen mirroring with touch, scroll, keyboard and device buttons for phones on this PC and on connected Linux hosts (bundled scrcpy-server v3.3.3, hash-verified).
- Add phones over Wi-Fi (Android 11+ Wireless debugging): network scan, pairing code, connect/disconnect, through this PC or a connected Linux host.

- Overview dashboard: live CPU, memory, disk, network and uptime for this PC and connected Linux hosts.
- Hosts show whether the host has a graphical session; Desktop shows the host's X11 session in a Senvakara pane with mouse and keyboard (requires x11vnc on the host; loopback-only, one-time password, stopped on close).

- Automatic reconnect of hosts after a connection loss (bounded backoff, stops on authentication problems or when you disconnect).
- tmux-backed SSH tabs that survive disconnects and reattach with ↻.

- Workspaces: save and reopen 1/2/4-panel layouts mixing terminals (incl. tmux), phone screens, logcat, files and remote desktops; panels that cannot run explain why and offer to connect the host.

- Send files straight from a Linux host to any phone, or from a phone to a Linux host, with progress and a size check — nothing is stored on this PC.

- Deploy: build a Flutter app on this PC and install it on any phone (including phones on another host), optionally launch it and show its log; step-by-step status, cancel, and run history.

- Network paths: give a Linux host extra VPN addresses (WireGuard, Tailscale, … that you already run; addresses seen on the host are offered with one click). Senvakara uses the LAN address when it can and falls back to a VPN address automatically when you are away, always requiring the same trusted host key; phones behind the host keep their identity, and ADB is never exposed.

- Automations: save recipes of steps (install/uninstall APK, launch app, wait, shell command, screenshot, Flutter build & install) for one phone, with explicit per-recipe permissions; every run shows all steps and the exact target for confirmation, stops at the first failure, can be cancelled and is kept in history.

### Changed

- Release builds unwind on panic, so a failure in one session does not close the whole app (NFR-02).
- Placeholder Senvakara app icon instead of the framework's default.

### Fixed

- Device availability shown after a restart no longer reflects the previous session.

## Versioning guidance

When the app is released, use entries such as:

```md
## [0.1.0] - YYYY-MM-DD
### Added
- ...
### Changed
- ...
### Fixed
- ...
### Security
- ...
```

Do not fabricate versions, release dates, benchmark results or working features.
