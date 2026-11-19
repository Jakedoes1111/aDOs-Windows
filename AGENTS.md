# aDOs — rules for every AI agent working in this repo

aDOs is a native desktop workspace (Rust + GPUI via the `gpui-kit` crate). Read `README.md`, then `docs/SPEC.md` and `docs/VISION.md`. Working on Windows? Read "Windows port" at the end first: some rules below are Linux-only.

## Hard rules
1. **Stay in scope.** Edit only what the task names. Do not edit the workspace `Cargo.toml`, `.cargo/` or `docs/SPEC.md` unless the task says so. Need something from another crate? Say so in your final note.
2. **Never guess an API.** GPUI and gpui-kit change often and your training data is wrong about them. Before using any type or method, find it in real source:
   - Get the reference clone once (it is not tracked and not a build input): `git clone https://github.com/longbridge/gpui-kit ref/gpui-kit`.
   - Guide: `ref/gpui-kit/skills/gpui-kit/SKILL.md` and its `references/` (read `coding-guides.md` "Rules for coding agents", `usage.md`, `recipes.md`).
   - Working examples: `ref/gpui-kit/examples/*` (dock, input, editor, markdown, hello_world, sidebar...), `ref/gpui-kit/crates/story/src`.
   - The exact versions the build uses: `~/.cargo/registry/src/*/gpui-kit-0.9.1`, `~/.cargo/registry/src/*/gpui-component-0.9.1`, `~/.cargo/registry/src/*/gpui-pre-0.3.12/src`.
   - Imports follow `use gpui_kit::*; use gpui_kit::component::...;` as in the examples.
3. **Build:** `cargo check -p <your-crate>` or `cargo build -p ados` from the repo root. Never run `cargo clean` or `cargo update`, or change profiles. Add dependencies only in the crate that needs them, and keep them few and light.
4. **GUI runs only on a virtual display**, never on the user's real desktop: `scripts/cloud-rig.sh` (Xvfb) or `scripts/rig.sh` (xspace, if installed). Subcommands: `up`, `shot <file.png>`, `act <xdotool args>`, `relaunch`, `status`, `down`. Look at your screenshot (read the PNG) before claiming anything renders. Stop the rig and every process you started when you finish.
5. **Tests use a throwaway data dir:** `ADOS_HOME=$PWD/target/test-home/<name>`, so real data in `~/.local/share/ados` is never touched. Screenshots and logs go under `target/test-out/<name>/`.
6. No `sudo`. Do not commit, push or switch branches unless the task says so.
7. No `unsafe` unless the task allows it. No `unwrap()` on I/O, parsing, or anything a user can cause; show the failure in the UI in a plain sentence.
8. Keep it small. No abstractions for later. A file over ~500 lines should be split.
9. The Browser keeps the focus contract in `docs/design/focus-contract.md`; run `scripts/focus-matrix.sh` after touching it. Shortcuts go through the registry in `docs/design/shortcuts.md`, never a direct key binding.

## Design rules (all visible UI)
- Midnight black, minimal, icon-first. Canvas `#030405`, panel `#08090C`, hairline border `#25282D`, text `#D4D7DF`, secondary text `#939BA8`, blue `#376BFA` (selection, actions, focus), red `#F04B55` (capture, attention). No other accent colours, no glow, no shadows beyond a subtle one on floating menus.
- Use `cx.theme()` tokens from gpui-kit for colours; the shell installs the midnight theme. Only hard-code the two brand colours when a token does not exist.
- Icons replace text where the meaning is clear; every icon-only control has a tooltip. Hit targets at least 32px. Radii 8px (10px for floating cards).
- Everything keyboard reachable. Visible focus. No hover-only controls. No praise copy, no fake durations.
- State survives: anything the user typed or arranged is saved without asking and restored on reopen.

## Finishing
Say what works, with the exact commands you ran and what you saw; what does not; what you need from other crates. Never write "works" for something you did not run and observe. If you could not finish, say exactly where you stopped.

## Low-memory machines (under about 12 GB)
- `cargo test` recompiles the whole GPUI dependency tree in a second profile and can exhaust memory. Run it on a bigger machine; put pure-logic tests in `#[cfg(test)]` and say they were not run.
- One cargo command at a time, with `-j 2`. No `--all-features`, no `--workspace`.
- If `/tmp` is RAM-backed, keep test data on disk under `target/`.

## Fresh Ubuntu machines, VMs and cloud sessions
- `scripts/cloud-setup.sh` installs the packages, the test rig and Servo; then `source ~/.bashrc`.
- `cargo test --workspace` and parallel jobs are fine with 15 GB. A cold `cargo build -p ados` takes about 6 minutes on 4 cores.
- If Servo cannot reach the internet there, browser tests use the local pages in `crates/ados-browser/fixtures/`, served at `http://127.0.0.1:8765/` by `cloud-rig.sh up`.
- With no agent logins the assistant shows "Can't reach pi", which is expected.

## Windows port
This repo also builds a native Windows 11 app from the same source. Plan and status: `docs/design/windows-port.md`. Where the rules above say otherwise, these win on Windows:
- **Build:** `cargo build -p ados --release` (every module; the Browser's Windows engine is `crates/ados-browser/src/webview/`).
- **Tests:** one crate per cargo call (the `standalone` examples collide at link time); see `.github/workflows/windows.yml`.
- **GUI:** there is no Windows virtual-display rig. The only UI check is CI's start-up job and its `smoke-screenshots` artifact. Never write that something works on Windows from a Linux machine.
- **Data:** default `%APPDATA%\ados`; for tests set `$env:ADOS_HOME` (PowerShell) to a folder under `target\test-home\`.
- **`unsafe`:** allowed for Win32 calls in Windows-only files (such as `crates/ados/src/single_windows.rs`), each with a `SAFETY:` comment.
- **Versions to read:** `gpui-pre-windows-0.3.12` for GPUI's Windows platform and `windows-0.62` for Win32, both under `~/.cargo/registry/src/`.
- **Shared code:** put Windows code behind `#[cfg(windows)]` or in new files, and keep Linux building (CI's `linux-check` job).
