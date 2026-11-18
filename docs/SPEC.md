# aDOs v1 — spec

> The v1 spec as written on 13 Nov 2026, kept for its design rules and module contracts. Where it differs from `README.md` and `VISION.md`, those are current: the assistant now talks to agent backends over ACP (not pi-mobile), Sessions, Tasks, Calendar, Settings and MCP exist, the module keys are Ctrl+1 to 8, and the Browser has tabs, tiles and an automatic Chromium switch. Written for Linux; Windows differences are in `docs/design/windows-port.md`.

One native window (Rust, GPUI through `gpui-kit` 0.9.1, Linux X11) for daily use.
Feel: Zed-fast, midnight black, icon-first. Reference pictures: `docs/design/reference/07-midnight-refined.png`
(shell + palette) and `docs/design/reference/04-inline-context-menu.png` (workspace + right-click menu).
Target machine: a low-end laptop (4 cores, 7 GB RAM, Intel HD 520). Idle cost must be near zero: no polling loops, no timers that repaint an idle window.

## What v1 is

| # | Piece | Crate | Done means |
|---|---|---|---|
| 1 | Shell | `crates/ados` (binary `ados`) | Window opens to the last workspace. 72px icon rail, 48px top bar, dock workspace. |
| 2 | Layouts | `crates/ados` | Modules split, tab, resize, move by drag. Right-click a module's tab/title for Move, Split, Float, Hide, Module settings, Save layout, Save as new layout. Named layouts switch from the top bar and the palette. No edit mode. |
| 3 | Command palette | `crates/ados-palette` | `Ctrl+K` anywhere. Two modes in one input: **Actions** (fuzzy over every registered action) and **Capture** (`Ctrl+Shift+K`, or Tab inside the palette). |
| 4 | Quick capture | `crates/ados-palette` + `ados_core::capture` | Type or paste anything, Enter saves it, type detected automatically (thought, URL, file, image). Shows in Notes → Inbox. |
| 5 | Notes | `crates/ados-notes` | Markdown files in `ADOS_HOME/notes`. List, search, edit, autosave, preview. Inbox view of captures. |
| 6 | Terminal | `crates/ados-terminal` | A real shell in a PTY: colours, cursor, resize, scrollback, copy/paste, vim/htop usable. |
| 7 | Browser | `crates/ados-browser` | Real web pages in a module: address bar, back/forward/reload, typing and clicking work. Servo first; Chromium fallback per site. |
| 8 | Ambient assistant | `crates/ados-assistant` | The Vivaldi "pi" experience inside aDOs: a small bubble always there, opens to a composer, grows to a reading card. Knows what the active module shows. Talks to an agent backend. |

Out of v1: canvas, visual flows, tasks module, phone companion, CEF, GPU texture sharing, settings UI beyond what a row above names.

## Shared contracts (`crates/ados-core`, already written — read the source)

- `paths`: all data under `ADOS_HOME` (default `~/.local/share/ados`).
- `capture`: `detect`, `save`, `list`, `delete`.
- `hub::ContextHub`: each module `publish`es a `Snapshot` (title, location, text, selection) when its content changes; the shell calls `set_active` on focus change. The assistant reads it.
- `actions::ActionRegistry`: modules and shell `register` palette actions in their `init`.
- `overlay::OverlayGuard`: floating UI calls `open`/`close`; the browser hides its child window while any overlay is up.
- `ids`: module ids, also used as dock panel names.

## Module crate contract

Each module crate is a library that exports:

```rust
pub fn init(cx: &mut App);                       // keybindings, palette actions, globals
pub struct XxxPanel;                              // implements gpui_kit::component::dock::Panel
impl XxxPanel { pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self>; }
```

`panel_name` is the matching `ados_core::ids` constant. The panel must restore its own state from
`ados_core::paths` when constructed (a panel can be closed and reopened, or rebuilt from a saved layout).
Each crate also ships `examples/standalone.rs`: a window holding just that panel, used for testing in xspace
(`ADOS_HOME=/tmp/ados-test-x cargo run -p ados-xxx --example standalone`).

The shell owns: window, theme, rail, top bar, dock, layouts, `register_panel` for every module, and mounting
the palette and assistant overlays above the dock.

## Shell detail

- Rail (72px, icons only, tooltips): Home, Notes, Terminal, Browser, Agent; bottom: Command (opens palette), Settings. Clicking a module icon focuses that module if it is in the layout, otherwise adds it to the centre. Active module icon is blue with a 2px indicator.
- Top bar (48px): layout name with dropdown (switch layout, `Modified` dot when unsaved changes), quiet outlined `Save layout` with a chevron for `Save as new…`. One hairline under it with the thin blue→red gradient.
- Default layouts shipped: **Research** (Browser left half, Notes top-right, Agent bottom-right), **Code** (Terminal + Notes), **Focus** (Notes only).
- Layout = `DockArea` dump as JSON in `paths::layouts_dir()/<name>.json`; last used layout + unsaved working state in `paths::state_file("shell")`, restored at start. A layout file that fails to load falls back to the default and says so in a notification; it is never deleted.
- Home module: recent notes, recent captures, layouts, "Resume" of the last layout. Minimal rows, no cards.
- Keys: `Ctrl+K` palette, `Ctrl+Shift+K` capture, `Alt+Space` assistant, `Ctrl+1..5` focus module, `Ctrl+W` close module, `Shift+Esc` zoom module, `Ctrl+S` save layout when no editor has focus.

## Assistant detail

Three sizes of one object, drawn above the dock:
1. **Bubble**: 24px circle, bottom-right by default, draggable, position saved. States: idle, running, reply ready, disconnected.
2. **Composer**: 440px card: header (drag, chat title, new chat, collapse), context chips, input, send/stop.
3. **Reading**: same card grows with the reply up to 80% of window height, then scrolls. Never yank the scroll while the user has scrolled up.

- Context chips: `This module` on by default (active module's `Snapshot`: title, location, text capped at 60k chars with a truncation note; selection as its own chip when present). `@` picks other open modules.
- Replies stream, render as markdown, code blocks have Copy. Tool calls show as a one-line steps strip (running/done/failed in plain words), minimizable.
- Backend (superseded): v1 used a private pi-mobile server; see `docs/design/backends.md` for the ACP backends that replaced it.
- Drafts and open/closed state survive restart. `Esc` collapses. `Alt+Space` toggles from anywhere in the window.

## Browser detail

v1 embeds a **real browser process as a child X11 window** inside the browser module's rectangle (reparent + resize to follow the pane; hide when the pane is hidden, the tab is not active, or `OverlayGuard::any`). Engine order: prebuilt `servoshell` if it can be driven usefully, otherwise a Chromium-family binary already on this machine in app/kiosk mode with a remote-debugging port for navigation, URL/title and page text. The module's own GPUI chrome (address bar, back/forward/reload, engine toggle) sits above the child window. Page title/URL/text go to `ContextHub`.

## Quality bar

- Each piece is tested by driving the real app on a virtual display with mouse and keys, with screenshots read by the tester.
- Nothing the user typed or arranged is lost on quit, crash or module close.
- A failure tells the user what happened and what to do, in one plain sentence.
