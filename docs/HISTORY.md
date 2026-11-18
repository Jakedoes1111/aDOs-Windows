# How aDOs was built

aDOs was written in twelve days of November 2026, almost entirely by AI coding agents working in parallel from written briefs, each ending with a handoff note and screenshots a tester agent had read.

| When | What |
|---|---|
| 6 Nov | A spike embedding Servo in a GPUI window. |
| 13 Nov | v1 overnight: shell, layouts, palette, capture, Notes, Terminal, Browser, assistant card, one agent per crate. |
| 13 Nov | Direction change (`VISION.md`): agents run natively behind one interface; no private services as dependencies. Agent backends over ACP, Sessions, the MCP server, Tasks and Calendar followed. |
| 13 to 15 Nov | Fourteen rounds of "tester drives the real app, fixer fixes" until v1.0 (tag `v1.0`: 2 known bugs, 0 blockers). |
| 15 Nov | Owner decisions after sign-off (`design/decisions-2026-11-15.md`). |
| 16 to 18 Nov | Browser work in stages: integration, the focus contract (`design/focus-contract.md`), tabs, tiled and follower tabs, Settings with zoom, the automatic switch to Chromium. |
| 18 Nov | Repo cleaned for sharing. |

## Decisions worth knowing
- **The browser engine is a separate process**, embedded as a child X11 window (prebuilt `servoshell` over WebDriver, or Chromium over CDP). Servo is not compiled in. An experiment that links Servo and paints pages as GPU textures exists outside `main` and is undecided.
- **The window manager draws the title bar**; aDOs draws none.
- **Zoom, not Float**, for giving one module the whole window (`Shift+Esc`).
- **One shortcut registry** (`design/shortcuts.md`); the page never wins over an aDOs chord.
- **Agents talk ACP**, through third-party adapters, so a new agent is a line in `backends.toml`.

## Windows port
From 18 Nov 2026 this repo (aDOs-Windows) ports the same source to Windows 11: plan, status and audit in `docs/design/windows-port.md`.

## Where the old material is
The per-round briefs, handoff notes, about 740 test screenshots, design explorations and the full git history before the cleanup are kept by the owner outside this repo (an archive folder with a git bundle of every branch). They are process records, not needed to build or extend the app.
