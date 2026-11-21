# UI/UX & Design System — Senvakara

**Status:** proposed guidelines, not an implemented interface. Desktop-first (Windows), expandable layout, keyboard-first, accessible states.

## 1. UX goals

- **One place, clear context:** show target device/host/connection route before sensitive actions.
- **Progressive disclosure:** main dashboard simple; advanced protocol settings in contextual panels.
- **Integrated workspaces:** terminals, files, Android, and desktops behave as first-class tabs/panels.
- **Honest feedback:** discovered ≠ trusted ≠ connected; permission denied has different UI from offline.
- **Efficient density:** development-tool information density without unreadable tiny text.
- **Performance:** no background video thumbnail streams; lazy mount of heavy panes.

## 2. Information architecture

Sidebar: **Overview**, **Hosts**, **Devices**, **Sessions**, **Files**, **Workspaces**, **Automations** (future), **Settings**. Search filters hosts/devices/workspaces. Global session tabs along top of content area; resizable main panel and optional inspector.

## 3. Main surfaces

**Overview:** grouped hosts + Android devices + recently used workspaces + active sessions; status/connection route always visible.

**Hosts:** Windows (local), Fedora (remote) and later others. Actions depend on capabilities: Terminal, Desktop, Files, Metrics, Pair/Manage.

**Devices:** Android A (Windows USB), B (Fedora USB), C (Wi-Fi ADB) shown **together**. Each card shows friendly label, host, transport, state; actions: Screen, Shell, Logcat, Install APK, Files (if available).

**Workspace:** tab/split grid with terminal, remote desktop, mirror, logcat. Reopen restores layout and reconnects eligible sessions but cannot revive dead nonpersistent processes.

**Activity panel:** ongoing transfers, builds, installs and automation jobs with target, progress, cancel, error and logs.

## 4. Sample navigation sketch

```text
┌────────────────────────────────────────────────────────────────┐
│ SENVAKARA    [Search devices / hosts…]           [Settings]   │
├──────────────┬─────────────────────────────────────────────────┤
│ Overview     │ Workspace: Flutter Testing                       │
│ Hosts        │ [Fedora SSH] [Android B] [Files]  [+]             │
│ Devices      ├────────────────────────┬────────────────────────┤
│ Sessions     │ Fedora terminal        │ Android B mirror       │
│ Files        │                        │ (active only on open)  │
│ Workspaces   ├────────────────────────┴────────────────────────┤
│ Automations  │ Transfer/build status & contextual inspector     │
└──────────────┴─────────────────────────────────────────────────┘
```

## 5. Visual tokens (initial proposal)

Use semantic tokens so theme can change; values below are starting proposals, not brand commitments.

| Token | Dark (default candidate) | Light |
|---|---|---|
| `background` | `#0F141B` | `#F6F8FA` |
| `surface` | `#18212B` | `#FFFFFF` |
| `surface-hover` | `#22303D` | `#EEF2F7` |
| `text-primary` | `#EAF0F6` | `#18212B` |
| `text-muted` | `#A5B3C2` | `#536273` |
| `accent` | `#69B8A9` | `#167A6B` |
| `error` | `#F07F7F` | `#B42318` |
| `warning` | `#F6C26B` | `#9A6700` |
| `success` | `#7DD5A5` | `#137A46` |

Typography: system UI font for interface, accessible monospaced font for terminal/paths. Base spacing 4/8 px increments; minimum comfortable targets ~32–40 px for desktop icons/buttons; 8 px radius default; visible focus ring. Respect reduced motion.

## 6. Components & states

- **HostCard:** name, OS, trust, last seen, endpoint labels, actions.
- **DeviceCard:** stable friendly name, host, USB/Wi-Fi route, debugging auth state, action menu.
- **ConnectionBadge:** online, offline, connecting, reconnecting, unauthorized, degraded; always text + icon (not color only).
- **SessionTabs:** close/duplicate/split, session health, unsaved work warning if relevant.
- **TransferJob:** source, destination, bytes, progress, cancel/retry (when safe).
- **PairingDialog:** who is being trusted, fingerprint/identity, scopes, cancel.
- **ErrorEmptyState:** diagnostic text, retry and documentation path.

## 7. Important interactions

1. Click Android B → show **Fedora / USB** → choose ADB Shell → request routed through Fedora; never silently fall back to Android A.
2. Install APK → confirm target device name/host and file → show progress + result; retry only if safe.
3. Remote Desktop → confirm host and endpoint → open integrated tab or label temporary external viewer in early build.
4. Unplug phone → route status flips offline; mirrored pane gives reconnect/close without freezing app.
5. User can explicitly select a different verified connection route; no implicit merge from identical device names.

## 8. Accessibility & localization

Keyboard navigation across sidebar/tabs/menus; ARIA labeling; screen-reader friendly statuses; 4.5:1 text contrast target for normal text; support international labels and future Indonesian/English i18n. Never use red/green alone to convey state.

## 9. Responsiveness

Minimum supported window size TBD. At narrow desktop widths, collapse inspector/sidebar, preserve action access. GPU-heavy sessions should render only visible streams where possible; avoid auto-playing all device thumbnails.

## 10. To validate with prototypes

Tab/split workspace interaction, embedded RDP/scrcpy surface, drag-and-drop APK targeting, appearance tokens, virtualized device list, keyboard shortcut conflicts with remote sessions, accessibility contrast.
