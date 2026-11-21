---
version: 1
slug: "apps-senvakara-ui-main-slint"
primary_target: "apps/senvakara/ui/main.slint"
related_targets: []
---

# Surface brief: Senvakara desktop app shell

Scope: the whole native app window (`apps/senvakara/ui`). Visitor mode: Operate.

Audience and job: developers, homelab/sysadmin/devops users and Android developers, weighted equally, at a desk for long sessions with several sessions open. They pick a host or device and open a terminal, files, mirror, logcat or desktop session on it, and must always know which machine and route a session touches.

Constraints: dark only; no emoji; native Windows title bar; standard controls; keyboard reachable; status never by color alone. Feature parity with v0.1.0 (checklist in the approved migration plan).

Owner decisions (2026-11-21): direction pinned to "in the manner of Termius, more minimal"; inventory sidebar plus session tabs; inspector panel instead of modals for details and forms.

Memorable moment: the route trail. A phone plugged into another computer shows its path, and the tab that mirrors it carries that computer's colour.

Unresolved: bundled monospace face (system Cascadia Mono during the spike); exact token values are fixed in `ui/theme.slint` and documented in DESIGN.md at finish.

## Direction contract

THESIS: A quiet inventory on the left, live sessions as tabs on top, and every session visibly tied to the machine it runs on. It refuses the nine-page dashboard where each page owns its own copy of Connect and state dies when you leave it.

OWN-WORLD: Deep slate ground with a darker shell (tab strip and sidebar) so chrome recedes; teal brand accent for the primary action and selection only; each host owns one hue from a fixed set of eight, used for its icon tile, its tabs' 2 px underline and its route chips; green, amber and red reserved for state and always paired with a word and a shape. Segoe UI Variable at 12, 13 and 15 px, rank carried by weight and tone rather than size; monospace only for terminals, paths, serials and fingerprints. 32 px rows, 4/8 spacing, 6 px radius, 1 px hairlines, no shadows except on overlays.

STORY: The user sees their machines and phones, understands at a glance what is reachable and through which path, opens what they need with one double-click, and trusts that nothing runs on the wrong device.

FIRST VIEWPORT: Native title bar. Below it a 36 px tab strip: pinned Home tab, then session tabs, then a new-session button. Under it a 208 px sidebar (Hosts, Devices, Workspaces, Automations; Settings at the bottom) and the work area: a search and quick-connect field with the primary "New host" button at its right, then compact two-line cards grouped under small "Hosts" and "Devices" headings, each with a 32 px icon tile, name, route subtitle and a status mark in a fixed right-hand cell. Selecting a card opens a 340 px inspector on the right.

FORM: The category standard played straight, as the owner pinned it (canon; the dealt assignment was index 3 and is overridden by the pin). Seed key 397c90b2. Raised by the declined hands: from the timetable, one small type scale with rank by weight and a state mark in a fixed cell; from the high-density web, compact rows that waste no vertical space. Signature move: the route trail, a chain "This PC > fedora (VPN) > Redmi 5A, USB" drawn with host-coloured segments on cards, inspectors and tab tooltips.

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance
