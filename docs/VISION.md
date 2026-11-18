# aDOs — direction (13 Nov 2026)

This replaces the assumption v1 was built on. v1 treated pi-mobile as a backend. That was a misreading.

## In the owner's words

- Ekaj (a personal memory service), pi-mobile (a phone client for the pi agent), the pi orb/overlay, the Vivaldi pi extension and Unity (a task and notes app), all the owner's earlier private tools, are **examples and references only**. They are not to be ported and must not be dependencies.
- The end goal is all of those features and capabilities **built directly into aDOs**, where aDOs is "the single, central and unified UI for all sessions".
- Multiple agent backends (pi, Claude Code, Codex, …) sit behind it; aDOs is "the only surface you interface with them from a user perspective".
- The small always-there assistant is "a sort of executive agent" that:
  - has context of every session across backends and harnesses, and of the projects being worked on;
  - has deep understanding of the aDOs program and GUI and can take action in it from natural-language prompts;
  - can modify parts of the UI on the fly, with hot reloading;
  - has context on the personal calendar/schedule, tasks, notes, etc.
- "All of this should run in aDOs natively, including the memory system, context engine, task management and orchestration."

## What this changes

| Area | v1 on 13 Nov | Direction |
|---|---|---|
| Assistant backend | Websocket client to pi-mobile on port 8791 | Native agent-backend layer: aDOs starts and owns agent processes itself (pi, Claude Code, Codex), one adapter each, no service in between |
| Sessions | Only pi-mobile's chats, shown in one card | A Sessions module: every session from every backend, live, resumable, in one list |
| Executive assistant | Chats with pi, gets the active module's text | Its own agent with tools over aDOs: read any module and session, run any palette action, change layouts, edit UI definitions |
| Acting in the GUI | Palette actions exist (`ActionRegistry`) | Expose the same registry to the assistant as tools; every action a human can run, it can run, visibly |
| UI changes on the fly | Compiled Rust only | Declarative, hot-reloaded UI layer for user-changeable surfaces (gpui-kit's `gpui-shell`: evaluated, adopt at step 6, see `docs/design/script-ui.md`) |
| Memory / context engine | None (Ekaj is external) | Native store inside aDOs: facts, session digests, project context |
| Tasks, calendar | None (Unity is external) | Native Tasks and Calendar modules feeding the context engine |

## Order proposed (each step leaves a usable app)

1. **Agent backends**: adapter trait plus the pi adapter (`pi --mode rpc`), replacing the pi-mobile client. Then Claude Code and Codex adapters.
2. **Sessions module**: list, open, resume, and watch sessions across backends.
3. **Assistant tools over aDOs**: read modules and sessions, run actions, switch and edit layouts.
4. **Native context engine and memory**, fed by notes, captures, sessions, terminal and browser.
5. **Tasks and calendar modules.**
6. **Hot-reloaded UI layer** the assistant can edit.

Status on 18 Nov 2026: steps 1, 2, 3 and 5 are built. Step 4 is in design (`docs/design/decisions-2026-11-15.md`). Step 6 exists as a trial panel (`ados-script`).
