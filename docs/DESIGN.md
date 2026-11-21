# Design System — Senvakara

**Status:** describes the native Slint interface in `apps/senvakara/ui` as built on 2026-11-22 (ADR-023). It replaces the earlier proposal written for the Tauri + React UI. The tokens below are read from `apps/senvakara/ui/theme.slint`; when they disagree, the file wins and this document is out of date.

Direction, pinned by the owner: in the manner of Termius but more minimal; dark only; no emoji or glyph characters as icons; clean, light, professional, easy to understand. The surface contract is in `.impeccable/surfaces/apps-senvakara-ui-main-slint.md`.

## 1. Principles

- **Always show the machine.** Every device, session and confirmation names the host and route it uses. The route trail ("RAFGT > USB") and the host's colour carry this.
- **Sessions are tabs; the inventory is Home.** Terminals, files, screens, logs, remote desktops, workspaces and runs live as tabs and keep running when the user looks elsewhere.
- **Details and forms in the inspector, not in modals.** A modal appears only when the user must decide before anything happens: confirmations, trusting a host key, pairing a phone.
- **Honest state.** State is a word plus a shape; colour only reinforces it. A disabled control says why.
- **Quiet chrome.** One small type scale, hairlines instead of boxes, no shadows except under overlays.

## 2. Structure

```text
+--------------------------------------------------------------------------+
| native Windows title bar                                                 |
+--------------------------------------------------------------------------+
| [Home] [session tab] [session tab] ... [+]                    tab strip  |
+-----------+------------------------------------------+-------------------+
| Hosts     | search and quick filter       [New host] | inspector         |
| Devices   | summary line                             | (selected item:   |
| Workspaces| Hosts    [card] [card]                   |  state, actions,  |
| Automations Devices  [card] [card] [card]            |  health, details, |
|           |                                          |  forms)           |
| Settings  | job list (transfers, installs)           |                   |
+-----------+------------------------------------------+-------------------+
```

- **Tab strip (38 px):** pinned Home, then one tab per session with an icon for its kind (terminal, screen, logcat, files, desktop, workspace, run). The active tab carries a 2 px underline in the colour of the host it runs on. Closing a running session asks first.
- **Sidebar (208 px):** Hosts, Devices, Workspaces, Automations; Settings at the bottom.
- **Inspector (340 px):** opens for the selected host or device. A form that has been typed into is never discarded without asking.
- **Command palette:** Ctrl+Shift+K. Window shortcuts use Ctrl+Shift (T new terminal, W close tab, C copy, V paste) so plain Ctrl keys reach the shell.

## 3. Tokens

| Role | Token | Value |
|---|---|---|
| Window chrome (tab strip, sidebar, inspector) | `shell` | `#0e1116` |
| Work area | `ground` | `#13161b` |
| Cards, fields, buttons | `surface` | `#1a1e25` |
| Hover | `surface-hover` | `#212630` |
| Selected | `surface-selected` | `#272e3a` |
| Terminal background | `term-ground` | `#0c0f13` |
| Hairline | `hairline` | `#262b34` |
| Control border | `hairline-strong` | `#343b47` |
| Text | `text` | `#e4e7ec` |
| Secondary text | `text-muted` | `#9ba3b0` |
| Hints, disabled | `text-faint` | `#7c8593` |
| Primary action, selection, focus | `accent` / `accent-ink` | `#5bc4ae` / `#06231d` |
| State | `success`, `warning`, `danger`, `info` | `#6ccb8e`, `#e3b34f`, `#f0807d`, `#6fa8f5` |

**Host hues** (one per host, by position in the host list; used for its icon tile, tab underline and route chips): `#5bc4ae`, `#6fa8f5`, `#a58bf0`, `#e487b8`, `#ee9a62`, `#d9be5a`, `#86ce77`, `#5cc3dc`.

**Type:** Segoe UI Variable for the interface at 12, 13 and 15 px; rank is carried by weight (400, 500, 600) and tone rather than size. Cascadia Mono only for terminals, paths, serials, fingerprints and logs.

**Metrics:** 32 px rows and controls, spacing 4 / 8 / 12 / 16 / 24, radius 6 px (10 px for dialogs), 1 px hairlines, 120 ms state transitions.

**Terminal palette:** 16 ANSI colours tuned to the theme in `apps/senvakara/src/term/palette.rs`; the 256-colour cube and true colour are passed through.

## 4. Components

All in `apps/senvakara/ui/components`. Every interactive one reacts to hover, press and keyboard focus (1 px accent border) and exposes an accessible role and label.

- **Controls** (`controls.slint`, `widgets.slint`): `Button` (primary, secondary, danger, ghost), `IconButton`, `TextField` (with error text and a masked variant), `BoundField`, `Select`, `CheckBox`, `Segmented`, `SearchField`, `ProgressBar`.
- **Status and text:** `StatusMark` (live = filled dot, idle = ring, busy = half-filled ring, attention = filled square, each with a word), `Notice` (danger, warning, info, success), `Reason` (why a control is unavailable), `DetailRow`, `MeterRow`, `GroupTitle`, `PageTitle`, `Hint`.
- **Lists:** `ItemCard` (icon tile, name, route or address, state in a fixed right-hand cell, context menu), `SidebarItem`, `ActionList`, `JobList`.
- **Overlays:** `DialogFrame`, `ConfirmDialog` (lists exactly what is affected), `WirelessDialog`, `PaletteOverlay`.
- **Session views:** `TermView`, `MirrorView`, `LogcatView`, `FilesView`, `DesktopView`, `RunView`, `WorkspaceView`, hosted by `PanelHost` so the same view works as a tab or as a workspace panel.
- **Icons:** monochrome 16 px stroke SVGs in `ui/icons`, tinted from `Theme` with `colorize`. No emoji or glyph characters.

## 5. Behaviour rules

- A control that cannot be used stays visible with a sentence explaining why.
- Every action that changes a phone shows the phone, the host and route, the serial and what will happen, and is bound to the serial the user saw.
- Destructive actions and anything that ends a running session ask first. Escape or a click outside cancels a dialog.
- Errors are sentences that name the problem and, where there is one, the way out. The raw backend code stays in the log.
- Heavy streams (screen, desktop, logs, metrics) run only while something shows them.

## 6. Accessibility

Keyboard reachable throughout, visible focus, text contrast of at least 4.5:1 on its surface for normal text, state never by colour alone, accessible labels on icon-only buttons. Not yet audited with a screen reader (QA-05).

## 7. Known gaps

- No light theme (owner decision).
- The title bar follows the Windows theme rather than being forced dark.
- Minimum window size 900 x 560 has not been checked on every screen.
- The monospace face is the system's Cascadia Mono, not a bundled font.
