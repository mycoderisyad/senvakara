# Design System — Senvakara

**Status:** describes the native Slint interface in `apps/senvakara/ui` as restyled on 2026-11-23 after a static analysis of Termius's shipped dark theme (ADR-023). It replaces the earlier proposal written for the Tauri + React UI. The tokens below are read from `apps/senvakara/ui/theme.slint`; when they disagree, the file wins and this document is out of date.

Direction, pinned by the owner: close to Termius (layout, type scale, surfaces and metrics were measured from its shipped stylesheets), with Senvakara's own accent; dark only; no emoji or glyph characters as icons; clean, light, professional, easy to understand. The surface contract is in `.impeccable/surfaces/apps-senvakara-ui-main-slint.md`.

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
| [Home] [session tab] [session tab] ... [+][v]                 tab strip  |
+-----------+------------------------------------------+-------------------+
| Senvakara | [New host] [search ..............]    (R)| inspector         |
| Hosts     |------------------------------------------| (selected item:   |
| Devices   | Hosts   1 of 2 hosts reachable           |  header, state,   |
| Keychain  | [card] [card] [card]                     |  actions, Health  |
| Workspaces| Group name                               |  and Details      |
| Workflows | [card] [card]                            |  cards, forms)    |
| Settings  | Activity (transfers, installs)           |                   |
+-----------+------------------------------------------+-------------------+
```

- **Tab strip (46 px, `shell`):** pinned Home and SFTP (as in Termius: SFTP shows the SFTP pane, opening one if none is open; that pane is not repeated in line), then one 32 px pill tab per session. A session tab starts with a 22 px tile filled with the hue of the machine it runs on and the icon of its kind (terminal, screen, logcat, files, desktop, workspace, run); the active tab is filled `surface-hover`, the others carry a faint white wash. Tabs that do not fit scroll sideways with the mouse wheel, and the active one is scrolled into view. Closing a running session asks first. "+" opens the New Tab page (search over recent connections, hosts, local shells, phones and SFTP; the pick takes the page's place in the strip); the chevron next to it is a menu of local shells, then hosts, then phones, whose first nine entries answer to Ctrl+Shift+1 to 9.
- **Sidebar (196 px, `surface`):** the product name, then Hosts, Devices, Keychain, Workspaces, Workflows; Settings at the bottom (Workflows replaced Automations, WF-IMPL-01). Workflows is a canvas between a module palette (left) and the inspector (right): cards carry a hue tile, title, category · machine and a state word with its shape; "runs on" links are dashed, order links solid, "uses the service of" links in `info`. Zoom scales positions and sizes in Rust (the software renderer cannot scale, ADR-028). Outline shows the same nodes as a keyboard list; the inspector's "Connect to" does what dragging a handle does. Items are 36 px, radius 10, 14 px medium labels with a count on the right.
- **Pages:** `PageToolbar` is a single row in the same order on every page: the page's own actions on the left, starting with its filled `ToolButton` (`+ New host`, `+ Add over Wi-Fi`, `+ New key`, `+ New workspace`, `+ New recipe`), then the compact search / SSH quick-connect field (Hosts, Devices), and Refresh on the far right; then a hairline, the page title (`SectionHeader`) and content with 24 px side padding. The add button is always there, also when the list is empty; the empty state (Workspaces, Hosts, Devices, Keychain) carries no button of its own and names the toolbar's in its sentence. The accent fill is kept for the one action that completes a form or triggers quick connect (Connect, Save, Build and deploy). Hosts and Devices are separate pages.
- **Inspector (352 px, `ground`):** opens for the selected host or device: a header with the hue tile, then state, actions and titled cards (Health, Details, Network paths). A form that has been typed into is never discarded without asking.
- **Command palette:** Ctrl+Shift+K. Window shortcuts use Ctrl+Shift (T New Tab page, 1 to 9 the nth entry of the "+" menu, W close tab, C copy, V paste) so plain Ctrl keys reach the shell.

## 3. Tokens

| Role | Token | Value |
|---|---|---|
| Tab strip, terminal background | `shell`, `term-ground` | `#141729` |
| Work area, inspector, fields inside cards | `ground`, `field` | `#1d2033` |
| Sidebar, cards, menus, dialogs | `surface` | `#282b3d` |
| Hover | `surface-hover` | `#32364a` |
| Selected | `surface-selected` | `#3e4257` |
| Hairline | `hairline` | `#31344a` |
| Control border | `hairline-strong` | `#43475c` |
| Text | `text` | `#f5f6fa` |
| Secondary text | `text-muted` | `#a1a5b8` |
| Hints, disabled | `text-faint` | `#868b9f` |
| Primary action, selection, focus | `accent` / `accent-hover` / `accent-ink` | `#5bc4ae` / `#7dd3c1` / `#062520` |
| State | `success`, `warning`, `danger`, `info` | `#5fcf8f`, `#f8aa4b`, `#f4676a`, `#58adf8` |

**Host hues** (one per host, by position in the host list): `#5bc4ae`, `#6fa8f5`, `#a58bf0`, `#e487b8`, `#ee9a62`, `#d9be5a`, `#86ce77`, `#5cc3dc`. Icon tiles are filled with the hue and carry a dark glyph (`tile-ink`, `#141729`), as Termius's OS tiles do.

**Type:** Inter 4.1 (Regular, Medium, SemiBold), bundled in `ui/fonts` under the SIL Open Font License; Termius's own face (CircularXX) is commercial. Sizes 12 (`text-small`), 14 (`text-body`, the base), 18 (`text-title`) and 22 (`text-display`); weights 400, 500 (`weight-medium`: names, labels, buttons, tabs, sidebar) and 600 (`weight-strong`: headings, primary buttons). Hierarchy comes first from tone (`text` against `text-muted`), then size, then weight. Cascadia Mono only for terminals, paths, serials, fingerprints and logs. (Before 2026-11-23 the family was given as "Segoe UI Variable", a name Windows does not register, so a fallback face was drawn; that was the "too thin" text.)

**Metrics:** 36 px rows, fields and buttons (30 px small), radius 10 (6 small, 14 for cards, 18 for dialogs), spacing 4 / 8 / 12 / 16 / 20 / 24 / 32, host cards 64 px high and at least 300 px wide with a 40 px tile, 2 px accent border on the selected card, 150 ms state transitions, menus with a 30 px soft shadow.

**Terminal palette:** 16 ANSI colours tuned to the theme in `apps/senvakara/src/term/palette.rs` (background `#141729`); the 256-colour cube and true colour are passed through.

## 4. Components

All in `apps/senvakara/ui/components`. Every interactive one reacts to hover, press and keyboard focus (1 px accent border) and exposes an accessible role and label.

- **Controls** (`controls.slint`; `widgets.slint`, which re-exports the files in `widgets/`): `Button` (primary, secondary, danger, ghost), `IconButton`, `TextField` (with error text and a masked variant), `BoundField`, `Select` (a themed field with a pop-up list; Up and Down change the pick), `CheckBox`, `Segmented`, `SearchField`, `ProgressBar`.
- **Status and text:** `StatusMark` (live = filled dot, idle = ring, busy = half-filled ring, attention = filled square, each with a word), `Notice` (danger, warning, info, success), `Reason` (why a control is unavailable), `DetailRow`, `MeterRow`, `GroupTitle`, `PageTitle`, `Hint`.
- **Forms** (`widgets/form.slint`): `IconField` (a field with a leading icon and an optional icon button at its end), `TextArea`, `Chip` and `ChipRows` (lines packed in Rust), `MenuRow` for pop-up lists, `LinkRow`, `ToolButton`. The host form and the key form group their fields in `Card`s. A pop-up list (`PopupWindow`) is used for the key menu of the host form, the "New key" chevron and the menu next to "+" in the tab strip.
- **Lists:** `ItemCard` (filled hue tile, name, route or address, state in a fixed right-hand cell, context menu), `SidebarItem`, `ActionList`, `JobList`.
- **Page layout** (`widgets/layout.slint`): `SectionHeader` (page title and state line), `EmptyState` (icon, title, sentence; no button), `PageToolbar`, `Card`, `CardGrid`, `GroupTitle`, `PageTitle`.
- **Overlays:** `DialogFrame`, `ConfirmDialog` (lists exactly what is affected), `WirelessDialog`, `PaletteOverlay`.
- **New Tab page** (`new-tab.slint`): `NewTabView` draws the `Launcher` global (search field, sections of 40 px rows with a `LaunchIcon` tile in the host's colour); it is hosted by `PanelHost` as kind `new-tab`.
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
