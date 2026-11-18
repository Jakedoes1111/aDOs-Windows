# Agent backends — design (step 1 of VISION.md)

Status: built (`crates/ados-agents`), checked against the real claude, pi and codex adapters on 13 Nov 2026.

## Decision

**aDOs is an ACP client.** ACP (Agent Client Protocol) is the open protocol Zed uses to host many agents in one editor. aDOs speaks ACP to every backend over stdio. There is no service in between and nothing from pi-mobile.

Evidence that this fits:

- The owner's Zed already runs `pi-acp`, `codex-acp` and `claude-acp` through it (`~/.config/zed/settings.json`, `agent_servers`).
- One Rust crate covers the client side: `agent-client-protocol` 5.2.0 (Apache-2.0). Its `process` feature uses `async-process` / `async-io`, the same smol family GPUI already runs on, so no second async runtime.
- The protocol already has what the Sessions module and the executive assistant need. Methods present in the crate: `session/new`, `session/prompt`, `session/cancel`, `session/list`, `session/load`, `session/resume`, `session/fork`, `session/close`, `session/delete`, `session/archive`, `session/set_title`, `session/set_mode`, `session/set_config_option`, `session/request_permission`, `session/update`, `fs/read_text_file`, `fs/write_text_file`, `terminal/create|output|wait_for_exit|kill|release`.

## Backends at launch

| Backend | How aDOs starts it | Version seen | Login |
|---|---|---|---|
| pi | `npx pi-acp@0.0.34` (wraps `pi`) | 0.0.34 | pi's own config in `~/.pi` |
| Codex | `npx @agentclientprotocol/codex-acp@2.6.0` | 2.6.0 | Codex's own login |
| Claude Code | `claude-agent-acp` (already in `~/.local/bin`) | 0.106.0 binary / 0.63.0 npm | Claude Code's own login |
| Gemini, Devin | from the same registry, free to add | — | their own |

Each agent keeps its own login and config. aDOs never stores agent credentials.

The list is data, not code: `ADOS_HOME/backends.toml` names each backend's command, args and env, so adding one needs no rebuild. aDOs can read Zed's registry format later to install adapters itself.

## Why not each agent's native protocol

pi has `pi --mode rpc`, Claude Code has `claude -p --output-format stream-json`, Codex has `codex exec --json` and an app server. Three clients to write and maintain, each with its own idea of sessions and permissions. ACP gives one client and one session model.

Cost of the choice: the adapters are third-party and young (`pi-acp` is 0.0.x). pi's native RPC exposes more (queue modes, compaction, thinking level, extension UI). **Keep a native pi RPC adapter behind the same trait as a fallback** if pi-acp is missing something Jake uses daily. Decide after step 1's checks, not before.

## What aDOs gets from hosting the agents' tools

ACP lets the client provide the agent's file and terminal access (`fs/*`, `terminal/*`). When an agent runs a shell command, aDOs can run it **in a visible aDOs Terminal panel**, and file edits go through aDOs, so the user watches and can take over. That is the "visible agent actions, easy human takeover" idea from the original product notes, without extra work.

`session/request_permission` becomes one aDOs approval UI (in the assistant card and the Sessions module), the same for every backend.

## How the executive assistant acts in aDOs

aDOs runs a **local MCP server** that exposes aDOs itself as tools:

- `actions.list` / `actions.run`: everything in the palette's `ActionRegistry` (open modules, switch and save layouts, new note, capture, …).
- `context.active`, `context.module`: what each module shows (`ContextHub` snapshots).
- `sessions.list`, `sessions.read`, `sessions.prompt`: every session across backends.
- `notes.*`, `capture.*`, and later `tasks.*`, `calendar.*`, `memory.*`.

ACP's `session/new` takes a list of MCP servers, so **every agent session started from aDOs can be handed this server**. The executive assistant is then just an agent session (on whichever backend Jake picks) that always gets it, plus a system prompt about aDOs. Any other session can be given it too, which is how "agents act in aDOs" stays one mechanism.

Every tool call is shown in the assistant's steps strip and is undoable where the action allows it.

## MCP transport

What each adapter accepts in `session/new` / `session/load` `mcpServers` (read from the published packages on 14 Nov 2026, `npm pack` of each version below):

| Adapter | stdio | http | sse | Source |
|---|---|---|---|---|
| claude-agent-acp 0.63.0 | yes | yes | yes | `dist/acp-agent.js`: advertises `mcpCapabilities {http, sse}`; maps entries without `type` to `{type: "stdio", command, args, env}` |
| codex-acp 2.6.0 | yes | yes | **no** (throws "Codex doesn't support MCP SSE transport protocol") | `dist/index.js` `normalizeMcpServer` |
| pi-acp 0.0.34 | accepted, **not used** | no | no | advertises `{http: false, sse: false}`; README: "MCP servers are accepted in ACP params and stored in session state, but not wired through to pi" |

**Choice: stdio.** It is the only transport ACP requires every agent to accept, and the only one both Claude and Codex take besides http. aDOs hands each session `{name: "ados", command: <aDOs binary>, args: ["mcp-bridge"], env: ADOS_MCP_SOCKET, ADOS_MCP_SECRET}`. The bridge is a ~60-line shim in the same binary: it connects to a Unix socket aDOs listens on (`$XDG_RUNTIME_DIR/ados/mcp-<pid>.sock`, else `ADOS_HOME/run/`; directory 0700, socket 0600), sends the per-launch 32-byte random secret as its first line, then copies stdio both ways. No TCP port exists, so nothing is reachable from another interface or another user; a connection without the secret is refused with a JSON-RPC error and closed.

**pi.** pi-acp drops the server, so pi sessions get no aDOs tools through ACP. Workaround until pi-acp wires it: pi's own MCP extension (pi-mcp-adapter) configured with `command: <aDOs binary>, args: ["mcp-bridge"]` and no env. Without env the bridge reads `current.json` (0600, next to the socket: socket path, secret, pid) written by the running aDOs. The assistant preamble still tells pi about aDOs.

Tool names use `_` where this document writes `.` (`actions.list` is `actions_list`): Anthropic and OpenAI tool names allow only letters, digits, `_` and `-`.

## Crate shape

- New crate `crates/ados-agents`: backend registry, process supervision (restart on crash, kill on quit), one ACP connection per running backend, session store, event fan-out into GPUI. No UI.
- `crates/ados-assistant` keeps its UI and swaps `client.rs` (pi-mobile websocket) for `ados-agents`. The pi-mobile client is deleted once parity is reached.
- New crate `crates/ados-mcp` later (step 3): the aDOs MCP server.
- New module `crates/ados-sessions` (step 2): the Sessions panel.

## Step 1 build plan, each with a check

1. `ados-agents` connects to `claude-agent-acp`, creates a session, sends a prompt, streams `session/update` text and tool calls. Check: a CLI example prints a streamed reply.
2. Same for `pi-acp` and `codex-acp`. Check: same example against each. Record anything pi-acp lacks against `pi --mode rpc`.
3. `session/list` + `session/load` per backend. Check: lists the user's existing sessions read-only.
4. Permission requests routed to a callback. Check: a tool that needs approval waits for it.
5. Assistant card switches to `ados-agents`, with a backend picker in its header. Check: tester round on the assistant, as before.
6. Remove the pi-mobile client and the `ADOS_PI_URL` setting.

## Testing

- Real-agent checks run **locally only**: cloud sessions have no agent logins.
- Cloud sessions test against a fake ACP agent. The crate ships one to copy (`examples/simple_agent.rs`).

## Unknowns to settle in step 1

- Whether `pi-acp` 0.0.34, `codex-acp` and `claude-agent-acp` each implement `session/list` and `session/load` (optional in ACP), and how they identify sessions created outside aDOs.
- Startup time of `npx` adapters; aDOs may need to install them once and run the binary directly.
- How each adapter reports cost and token use, if at all.
- Whether the crate's `process` feature plays well with GPUI's executor in practice (it should: same smol family).
