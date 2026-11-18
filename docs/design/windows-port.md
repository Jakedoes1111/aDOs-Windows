# Windows port — plan and notes

Native Windows 10/11 build of aDOs from the same tree as Linux. Rules and order: `CLAUDE.md`.
Status lines say what was **compiled** and what was **run**; nothing here has run on Windows yet.

Versions read for this plan (sources under `~/.cargo/registry/src/`, or downloaded from crates.io):
gpui-kit 0.9.1 → gpui-pre 0.3.12 and gpui-pre-windows 0.3.12; alacritty_terminal 0.26.0;
raw-window-handle 0.6.2; webview2-com 0.39.1 and webview2-com-sys 0.39.1.

## 1. Linux-only code, crate by crate

| Crate | File | Linux-only call | Windows plan (step) |
|---|---|---|---|
| ados | `src/single.rs` | `os::linux::net::SocketAddrExt` abstract socket, `os::unix::net::UnixListener/UnixStream` | Step 1: `single_windows.rs` stub (no guard). Step 2: `File::try_lock` (portable) + a named pipe `\\.\pipe\ados-<hash>` for "raise" |
| ados | `src/main.rs` | none; `WindowDecorations::Server` is fine on Windows | none |
| ados-core | `src/paths.rs` `home()` | `$XDG_DATA_HOME`, `$HOME/.local/share` | Step 2: `%APPDATA%\ados` under `cfg(windows)`; `ADOS_HOME` still wins |
| ados-core | `src/capture.rs:83` | `~/` expansion through `$HOME` | Step 2: `%USERPROFILE%` |
| ados-agents | `src/config.rs:79` `ados_home()` | same rule as `paths::home` | Step 2: same change |
| ados-agents | `src/config.rs:58,65` default `command = "npx"`, `src/launch.rs` `npx_spec` / `npx_caches` (`$HOME/.npm`) | Rust's `Command` on Windows does not find `npx.cmd` from `npx`; npm cache is `%LOCALAPPDATA%\npm-cache` | Step 2: `npx.cmd` on Windows, accept both names in `npx_spec`, add the Windows cache dir |
| ados-assistant | `src/ui/agent.rs:26` | `$HOME` as working dir | Step 2: `%USERPROFILE%` |
| ados-calendar | `src/subs.rs:75,91` | `$HOME`; runs `curl` | `curl.exe` ships with Windows 10 1803+; only the `$HOME` fix |
| ados-mcp | `src/server.rs` | `UnixListener`, `PermissionsExt` 0700/0600, `OpenOptionsExt::mode`, `/proc/<pid>` liveness | Step 3: named pipe with a DACL for the current user only; keep the per-launch secret; pid liveness via `OpenProcess` |
| ados-mcp | `src/bridge.rs` | `UnixStream::connect` | Step 3: open the named pipe |
| ados-mcp | `Cargo.toml` | hard dependency on `ados-browser` (`browser_tools.rs`) | Step 3: optional `browser` feature, default on |
| ados-terminal | `src/pty.rs:152` | `$SHELL`, `/bin/sh -i` | Step 4: `tty::Shell::new("powershell.exe", [])` (or `pwsh.exe` if on PATH) |
| ados-terminal | `src/pty.rs:187,259` | `/proc/<pid>/stat` foreground-job check | Step 4: no foreground-job check on Windows (ConPTY hides it); ask before close instead or skip |
| ados-terminal | `src/panel.rs:391` | `OpenOptionsExt::mode(0o600)` on history file | Step 4: plain write; the file is under the user's profile |
| ados-terminal | alacritty_terminal `tty::new` | none: 0.26.0 has `tty/windows` (ConPTY) | Step 4: check `tty::Options` fields used are present on Windows |
| ados-browser | `engine.rs` | `/usr/bin/setpriv`, `CommandExt`, rustix `kill_process_group`, XDG dirs | Not used with WebView2 |
| ados-browser | `runtime.rs` | `UnixStream::pair` wake pipe, `rustix::event::poll` | Replaced by the WebView2 runtime (below) |
| ados-browser | `driver.rs`, `protocol.rs` | x11rb, Servo WebDriver / Chromium CDP over TCP | Replaced: WebView2 events and `ExecuteScript` |
| ados-browser | `xembed.rs`, `cutout.rs`, `shot.rs` | X11 child window, XShape cutouts, XTest, X key grabs, X image grab | Replaced: child HWND via WebView2 controller, `SetIsVisible`, `AcceleratorKeyPressed`, `CapturePreview` |
| ados-browser | `focus.rs:72`, `engine_keys.rs` | keysyms via `xembed::chord_keysym` | Windows: virtual-key codes from `AcceleratorKeyPressed` |
| ados-browser | `fallback.rs` | Chromium on PATH | Not needed: WebView2 is Chromium |
| ados-script | `gpui-shell` with `quickjs` | C build of QuickJS | Not in the `ados` binary; not checked for Windows |

Portable as they are (no OS calls found): ados-notes, ados-tasks, ados-palette, ados-sessions, the rest of ados-calendar, the shell code in `ados`.

## 2. The Browser seam

`ados-browser` is already split at one struct: **`runtime::Runtime`**.
The panel side (`lib.rs`, `tabs.rs`, `tab_strip.rs`, `tiles.rs`, `tile_view.rs`, `nav.rs`, `state.rs`, `view.rs`, `visibility.rs`, `park.rs`, `open.rs`, `hints.rs`, `page_hints.rs`, `comment*.rs`, `control.rs`) talks to the engine only through:

- `Runtime::start(parent, engine, tab, url, native_keys, hints) -> (Arc<Runtime>, Receiver<Event>)`
- `Runtime::{command(CommandMsg), geometry, visible, overlay, cutout, focus_page, focus_address, typed, shortcuts, shortcut_done, shot, repaint, stop}`
- the enums `Event` and `CommandMsg`, plus `xembed::Rect`, `cutout::Holes`, `shot::{path_for, to_window}`.

Plan: **a cfg-split module, no trait.** `lib.rs` keeps `mod runtime;` and Windows gets
`#[cfg_attr(windows, path = "webview/runtime.rs")]` with the same public items. Engine files
(`driver.rs`, `engine.rs`, `protocol.rs`, `xembed.rs`, `cutout.rs`, `shot.rs`, `follow.rs`, `fallback.rs`)
become `#[cfg(unix)]`; `Rect` and `Holes` move to a small shared file so both runtimes use them.
Linux code stays where it is, so merges from `linux/main` still apply.

How each piece maps on Windows:

| Linux runtime | WebView2 runtime |
|---|---|
| Engine process + driver thread + X11 thread, woken by a Unix pipe | Everything on the GPUI UI thread (WebView2 requires its creating STA thread). No extra threads. Events go to the panel through the same `async_channel` |
| One engine, many tabs inside it | One `ICoreWebView2Controller` per tab (tiles = several visible controllers), all sharing one `ICoreWebView2Environment` |
| `geometry` → X11 move/resize | `controller.SetBounds(RECT)` in physical pixels (GPUI scale factor × logical bounds) |
| `visible` / `overlay(true)` → unmap | `controller.SetIsVisible(false)` (WebView2 windows are real child HWNDs and paint above GPUI) |
| `cutout` (XShape holes for floating cards) | No shaped holes: hide the page while a card overlaps it, or use `SetWindowRgn` on the WebView's child HWND (to try) |
| Key grabs; aDOs chords win (F9) | `controller.add_AcceleratorKeyPressed`: chords in the user's keymap → `SetHandled(true)` and send `Event::Shortcut`; `settings.SetAreBrowserAcceleratorKeysEnabled(false)` drops the engine's own shortcuts |
| `focus_page` / `focus_address` | `controller.MoveFocus(PROGRAMMATIC)` / `SetFocus(gpui hwnd)`; `add_GotFocus`/`add_LostFocus` for F3/F7 |
| `Navigate/Back/Forward/Reload` | `webview.Navigate`, `GoBack`, `GoForward`, `Reload` |
| `Page` snapshot (url, title, text) | `add_SourceChanged`, `add_DocumentTitleChanged`, `add_NavigationCompleted`, then `ExecuteScript` for text |
| `Opened` (popups) | `add_NewWindowRequested`: create a new controller, `args.SetNewWindow(new_webview)`, emit `Opened` |
| `Closed` (`window.close()`) | `add_WindowCloseRequested` |
| `Eval` | `ExecuteScript` (returns JSON text) |
| `Screenshot` / `shot` | `CapturePreview(PNG, IStream)`, crop with the existing `png` crate |
| `hints.js`, `comment.js` | `AddScriptToExecuteOnDocumentCreated` + `ExecuteScript`; messages back through `add_WebMessageReceived` instead of polling |
| Follower tabs (polled over WebDriver) | `WebMessageReceived` from the page script: event-driven, no 250 ms poll |

## 3. WebView2 binding: `webview2-com` 0.39.1 (chosen)

Read: its `Cargo.toml`, `src/callback.rs`, `examples/sample.rs`, and `webview2-com-sys` `build.rs` and `bindings.rs`.

- It depends on `windows` **0.62**, the same version GPUI's Windows platform uses (`gpui-pre-windows` 0.3.12), so one `windows` crate in the build.
- It ships `WebView2LoaderStatic.lib` for x64/x86/arm64 and links it statically: no `WebView2Loader.dll` to ship.
- The bindings have every call the port needs: `CreateCoreWebView2Controller` (child of an HWND), `SetBounds`, `SetIsVisible`, `MoveFocus`, `add_AcceleratorKeyPressed`, `SetIsBrowserAcceleratorKeyEnabled`, `add_NewWindowRequested`, `ExecuteScript`, `CapturePreview`, `CallDevToolsProtocolMethod`.
- `callback.rs` gives typed completion and event handlers (`…CompletedHandler::create`, `wait_for_async_operation`).

Rejected:
- `wry` 0.57.2: builds on webview2-com, adds a cross-platform layer aDOs does not use (the Linux side is Servo), hides accelerator keys and `CapturePreview` (reachable only through its `controller()` escape hatch). Two layers where one does.
- `webview2` 0.1.4: last release 2021, `winapi` based.

## 4. The window handle on Windows

GPUI's `Window` implements `raw_window_handle::HasWindowHandle` (raw-window-handle 0.6).
On Windows `gpui-pre-windows` 0.3.12 `src/window.rs:597` returns `RawWindowHandle::Win32(Win32WindowHandle { hwnd, .. })`.
`ados-browser/src/lib.rs` already matches `Xlib`/`Xcb` there; Windows adds a `Win32(h)` arm giving `HWND(h.hwnd.get() as *mut _)` as the WebView2 parent.

Things read in GPUI's Windows window that matter:
- The top-level window has `WS_EX_NOREDIRECTIONBITMAP` (DirectComposition) unless `GPUI_DISABLE_DIRECT_COMPOSITION` is set (`platform.rs:251`). WebView2 child HWNDs should still composite above it; this is the first thing to confirm on a real Windows machine.
- The window style has no `WS_CLIPCHILDREN`; GPUI draws through DirectX, so this is expected to be harmless. Unverified.
- `build.rs` compiles HLSL shaders with `fxc.exe` only in release builds on a Windows host; a debug `cargo check` from Linux skips it. The CI release build needs the Windows SDK (present on `windows-latest`).

## 5. Step 1 (this commit series)

Goal: `ados` type-checks for `x86_64-pc-windows-msvc` with Browser and Terminal compiled out; Linux unchanged.

- The `ados` crate already has one feature per module. Windows builds use
  `--no-default-features --features notes,assistant,palette,calendar,sessions,tasks`.
  MCP is left out too until step 3 (it needs `ados-browser` and Unix sockets).
- `single.rs` gets a Windows stub (no one-window guard) until step 2.
- CI: `.github/workflows/windows.yml` on `windows-latest`, on push to `main` and on demand.

Results: see "Status" at the end.

## Status

### Step 1 (18 Nov 2026)

Compiled on Linux (this machine, toolchain 1.99.1, debug `cargo check`, no linking):
- `cargo check -p ados --target x86_64-pc-windows-msvc --no-default-features --features notes,assistant,palette,calendar,sessions,tasks`: passes, 9 warnings. The warnings are dead code that is only used with `browser`/`notes` on (`page_actions.rs`, `hints.rs:136`, `modules.rs:172`, `shell.rs:447`); Linux with default features has none. Left alone to keep shared files unchanged.
- No C toolchain or Windows SDK was needed for the check: GPUI's `fxc.exe` shader step runs only in release builds on a Windows host.
- `cargo check -p ados` (Linux, default features): passes, 0 warnings.

Errors the excluded crates give for the Windows target today (`cargo check -p <crate> --target x86_64-pc-windows-msvc`, partial run):
- ados-terminal, 4 errors: `pty.rs:166` `Pty::child()` is Unix-only in alacritty_terminal; `panel.rs:391,398,401` `os::unix` file modes.
- ados-mcp: stops on its `ados-browser` dependency first (6 errors: `engine.rs:4,6,122` rustix process and `process_group`; `runtime.rs:13,400,515` `UnixStream` and timespec types). `x11rb` itself compiles for Windows. ados-mcp's own Unix-socket errors were not reached.
- ados-browser: not checked on its own (stopped; same errors as above expected).

CI (`.github/workflows/windows.yml`, `windows-latest`), first run 44281068275 on commit 4198af7:
- **Release build of `ados.exe` with MSVC: green** (22 min cold; includes GPUI's `fxc.exe` shader step). Artifact `ados-windows-x64` (13.6 MB zip).
- Tests (not gating): the first test binary passed 9 of 9. `ados-agents/tests/fake_agent.rs` then failed 7 of 8 because it looked for `examples/fake_agent` without `.exe`; cargo stopped there, so the other crates' tests did not run. Fixed with `EXE_SUFFIX`, and the workflow now uses `--no-fail-fast`; the next run shows the full test picture.
- The `standalone` examples in several crates collide on one output name (cargo warning, not an error).

Never run: any Windows binary, any test on Windows (CI runs them without gating), the UI on Windows.

From now on compiles go to cloud sessions or CI, not the owner's laptop.

## Open questions for the owner

1. Windows 10 support: is Windows 11 only acceptable (WebView2 ships with it), or must Windows 10 work, with the Evergreen runtime installer?
2. Data folder: `%APPDATA%\ados` (roams with the profile) or `%LOCALAPPDATA%\ados` (machine-local, better for large caches)? The plan assumes `%APPDATA%`.
3. Terminal default shell: Windows PowerShell 5 (`powershell.exe`, always present) or PowerShell 7 (`pwsh.exe`) when installed?
4. Floating cards over a page: Linux cuts a hole in the page; on Windows the first version hides the page while a card overlaps it. Acceptable for a first version?
5. When will a Windows machine or VM exist for real UI checks?
