# ACP and agents

## Which side is the tool?

The app is the ACP **client** (the role editors like Zed play). It launches an agent as a subprocess and talks JSON-RPC 2.0 over stdin/stdout. It is not an ACP agent. An ACP agent is something an editor launches; this tool wants to do the launching, with its own canvas.

## Basic message flow

- `initialize` (negotiate version and capabilities), `authenticate` if required.
- `session/new` (with a working directory) or a way to reuse a session.
- `session/prompt` to send a prompt; the agent streams `session/update` notifications (message chunks, thought chunks, tool calls and updates, plans, mode changes).
- The agent may call back: `session/request_permission`, file reads/writes, terminals.
- `session/cancel` interrupts; the turn ends with a stop reason on the `session/prompt` response.

## Why ACP over headless CLI

Headless CLI runs (for Claude Code: `claude -p`, `--output-format stream-json`, `--resume`, `--fork-session`) work, but unattended agents cannot ask permission mid-run, so people end up disabling permissions. The author also reported issues trying headless mode. ACP gives permission callbacks (the hook for approval gates), cancel, typed streaming events, and a long-lived session.

## Agent support

All of these go through ACP; some natively, some via adapters. This is a snapshot (late 2026) taken from the `acp-cli` registry; verify before depending on it.

- **Native ACP:** Gemini CLI (`gemini --acp`), GitHub Copilot (`copilot --acp --stdio`), Cursor (`cursor-agent acp`), Goose (`goose acp`), Kiro (`kiro-cli-chat acp`), Kimi (`kimi acp`), Qwen Code (`qwen --acp`), Droid, OpenClaw, and newer entries such as iFlow, Qoder, Trae.
- **Via adapter or launcher:** Claude (`@agentclientprotocol/claude-agent-acp`; older name `@zed-industries/claude-agent-acp` also appears), Codex (`@zed-industries/codex-acp`), OpenCode, Kilocode, Pi.
- Claude Code and Codex do not speak ACP natively. A feature request for native Claude Code ACP exists on its issue tracker; no committed plan was found. OpenAI has not published an official ACP adapter; Zed's is the de facto one.
- Each adapter uses the login of the underlying CLI, so the app never handles credentials. Adapters are npm packages, so Node.js is needed (Codex also ships release binaries).
- Zed's ACP Registry is a curated list of agents. `acp-cli` (Rust, a port of ACPX) is worth reading for spawn, session, and permission-mode handling.

Design consequence: an adapter layer needs only a "command to spawn" per agent. If an agent later ships native ACP, change the command and keep the rest. Start with two agents (Claude via adapter, Gemini natively) to keep the interface clean.

## Session reuse

Reuse is optional per agent. Check capabilities at `initialize` (default is `loadSession: false`).

- `session/load`: restores context and replays history to the client (handle replay so it is not drawn as new output).
- `session/resume`: continues a session without replay; meant for agents that do not store full history.
- `session/list`: lists sessions the agent knows.
- `session/close`: cancels work and frees resources.
- `session/fork`: unstable; behind a feature flag in the Rust crate and an unstable capability in the spec.
- v2 direction: `list`, `resume`, `close` become required for agents that advertise sessions; replay folds into `resume`; fork stays optional.

Selection order for a phase needing earlier context: fork if supported, else resume or load, else fresh session seeded from the plan.

Caution to test: a spec passage suggests reloads work when the request's working directory matches the session's. With a separate jj workspace per phase, resuming from a different directory may not work.

## Diffs from the agent

Tool calls of kind `edit` can carry diff content (file path, old text, new text). The agent has to emit these; test that the Claude adapter does. A v2 proposal reshapes this to explicit add, delete, modify, move, and copy operations.

## Permissions

Use `session/request_permission` as the hook for approval gates. Combine with a separate jj workspace per phase so a bad run is easy to throw away.
