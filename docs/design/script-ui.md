# Hot-reloaded UI (step 6 of VISION.md): gpui-shell evaluation

Result of the cloud evaluation on 13 Nov 2026. Nothing was changed in the repo. The spike is not kept.

## Verdict

**Adopt later, at step 6, after a small one-panel trial.** Do not make it the base for the assistant's UI editing yet.

## What was shown working (crates.io `gpui-shell` 0.9.1, the same `gpui-pre =0.3.12` as aDOs)

| Check | Result |
|---|---|
| Hot reload | Edit JS → window changes in 0.3–0.6 s (`--watch`, or `ShellRuntime::watch(Entity<ShellRoot>)` when embedded) |
| Embedding | `gpui_shell::dock::ScriptPanel` sits in a gpui-kit `DockArea` beside a native panel |
| Host API | A host `HostModule` (`ados`) gave data to JS, and a JS click ran a Rust action (`palette.toggle`) |
| Syntax error | Old view stays; an error banner appears; the host keeps running |
| Infinite loop | Render is stopped at 2 s and an event handler at 5 s; the host keeps running |
| Sandbox | File, network and process access are denied unless the manifest grants them |
| Cost | About 270–300 MB RSS for the whole process (debug build), idle CPU 0–3 % |

## Before adoption

- A reload discards in-memory panel state (an unsaved draft was lost). The script must save state through host storage.
- The capability prompt and approval layer is not built. aDOs must own approval.
- The crate says it is experimental; its interfaces can change between minor versions. Pin it exactly.
- Its default `webview` feature switches GPUI to `gpui-fast`. Use `default-features = false`.
- Not tested with aDOs's Rust 1.99.1 pin (the test used 1.97.0).

## Shape when adopted

A crate `ados-script` (a module, depends only on `ados-core`) registers one `HostModule` named `ados` that exposes `ActionRegistry` and `ContextHub` reads to scripts. Script panels live in `ADOS_HOME/ui/<name>/`. The assistant edits those files through the aDOs MCP server; the watcher reloads them.
