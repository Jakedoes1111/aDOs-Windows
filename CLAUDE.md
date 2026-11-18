# aDOs for Windows — context for every session in this folder

This folder is the **native Windows port** of aDOs. Read `README.md`, `AGENTS.md`, `docs/SPEC.md`, `docs/VISION.md` and `docs/design/` first; they describe the app and its rules (design rules, "never guess an API", focus contract, shortcut registry). Where this file and `AGENTS.md` differ, this file wins here.

## Where things are
- This repo: `~/Projects/ados-windows`, remote `origin` = github.com/Jakedoes1111/aDOs-Windows (public). Work and push here only.
- The Linux app: `~/Projects/aDOs`, GitHub Jakedoes1111/aDOs. **Never edit, build in, or push to it.** It is available here as the read-only remote `linux` (push is disabled on purpose): `git fetch linux` then merge `linux/main` to pick up Linux changes.
- This repo started as a copy of the Linux repo's `main` at its first clean commit, so merges from `linux/main` work. Keep them working: do not reformat or move shared files without need; put Windows code behind `#[cfg(windows)]` or in new files so both systems build from the same tree. The long-term aim is that this port can be merged back, not a permanent fork.

## The goal
A native Windows 10/11 build of aDOs with every module, using **WebView2** for the Browser (the owner's decision). Linux must keep building from the same source.

## What the port needs (from reading the source; nothing here was test-built)
| Part | State on Windows | Work |
|---|---|---|
| Window, panels, layouts (GPUI via `gpui-kit` 0.9.1) | GPUI supports Windows | Confirm the pinned versions build with MSVC |
| Notes, Tasks, Calendar, palette, capture, Sessions, Settings | Plain files and UI | Expected to work |
| Data folder (`ados-core/src/paths.rs`, default `~/.local/share/ados`) | Linux convention | `%APPDATA%\ados` on Windows; `ADOS_HOME` still overrides |
| One running copy (`ados/src/single.rs`) | Unix lock | Windows equivalent (named mutex or lock file) |
| Agents (`ados-agents`, started through `npx`) | Program names and paths differ | `npx.cmd`, path handling |
| Agent tools connection (`ados-mcp`: Unix socket + per-launch secret) | No Unix socket by default | Named pipe (or AF_UNIX on recent Windows); keep the secret check |
| Terminal (`ados-terminal`; `alacritty_terminal` 0.26 supports ConPTY; `pty.rs` uses Unix calls) | Needs a Windows path | ConPTY through alacritty_terminal's Windows side; default shell PowerShell |
| Starting the engine (`ados-browser/src/engine.rs`, via `/usr/bin/setpriv`) | Linux only | Not needed with WebView2 |
| **Showing the page** (`ados-browser/src/xembed.rs`, `runtime.rs`, `cutout.rs`, `shot.rs`: an X11 child window, x11rb) | Linux only | **WebView2** hosted as a child of the GPUI window (HWND), positioned to the Browser pane, hidden under overlays |
| Keys while a page has focus (X11 key grabs) | Linux only | WebView2 accelerator-key events, so aDOs chords always win (focus contract F9) |
| Page tools (read text, click, type, screenshot, link hints, comments; today over WebDriver/CDP) | Engine-specific | WebView2 `ExecuteScript`, DevTools protocol calls and `CapturePreview`; `hints.js` and `comment.js` are plain JS and should carry over |

The Browser keeps its GPUI chrome (tab strip, address bar, tiles, follower tabs); only the engine layer under it changes. Look for the seam between `ados-browser`'s panel code and its engine code before designing; prefer one small trait or cfg-split module over a parallel crate. Tabs = one WebView2 per tab or tile. Popups (`NewWindowRequested`) become tabs, as on Linux.

## How to verify (this machine is Linux)
- You cannot run a Windows binary here. Do not claim anything "works on Windows" from this machine.
- Type-check for Windows locally where possible: `rustup target add x86_64-pc-windows-msvc` then `cargo check -p <crate> --target x86_64-pc-windows-msvc` (crates with C dependencies may need the MSVC SDK; `cargo-xwin` can supply it; find out, do not assume).
- Real builds and tests: a GitHub Actions workflow on `windows-latest` in this repo (build, `cargo test`, upload the `.exe` as an artifact). Build on push to `main` and on demand, not on every branch push, to keep runs short.
- Real UI checks need a Windows machine or VM; the owner will say when one exists. Until then say exactly what was compiled and what was never run.
- Linux must still pass: `cargo check -p ados` here after every shared-code change.

## Machine rules (the owner's laptop)
- One cargo command at a time on the whole machine, with `-j 2`; the Linux repo may also be building. No `cargo test` locally (memory), no `cargo clean`, no `cargo update` unless a Windows dependency needs it and you say so.
- This folder has its own `target/`; it is large, never commit it.
- No sudo. No GUI on the owner's real desktop.
- Never guess a crate API (WebView2 bindings, the `windows` crate, GPUI's Windows platform, raw window handles): read the real source under `~/.cargo/registry/src/` or the crate docs first, and say which version you read.
- Commit in small steps to `main` or short branches of this repo; commits use the repo's no-reply identity already configured. Never put personal details, keys or machine paths into the repo; it will be public.

## Suggested order (each step leaves Linux building)
1. Make the workspace type-check for the Windows target with the Browser and Terminal compiled out there (cfg gates and a build feature), and add the Windows CI workflow. First green Windows build = first milestone.
2. Data folder, single-instance lock, agent program names.
3. Agent tools connection over a named pipe.
4. Terminal on ConPTY.
5. Browser on WebView2: one page in the pane, then navigation and page reading, then tabs and tiles, then keys and the focus contract, then page tools.
6. README section "Install on Windows" (WebView2 runtime ships with Windows 11; Windows 10 may need the Evergreen installer), then an installer and signing, last.
