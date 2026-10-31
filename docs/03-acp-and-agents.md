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
- **Via adapter or launcher:** Claude (`@agentclientprotocol/claude-agent-acp`; older name `@zed-industries/claude-agent-acp` also appears), Codex (`@agentclientprotocol/codex-acp`; replaces `@zed-industries/codex-acp`), OpenCode, Kilocode, Pi.
- For Codex, the tested adapter translates ACP into Codex App Server operations. No modification to Codex itself is required. See the [Codex spike results](12-codex-acp-spike.md) for the tested version and observed behavior.
- Authentication belongs to the selected agent/provider, not ACP. The Codex spike reused an existing ChatGPT login; other agents and installations must be checked rather than assuming universal credential reuse. The Codex npm adapter includes a compatible Codex dependency.
- Zed's ACP Registry is a curated list of agents. `acp-cli` (Rust, a port of ACPX) is worth reading for spawn, session, and permission-mode handling.

Current first target: **Codex through ACP**, using a user-installed executable.
Waypoints takes the client role itself; it does not control Zed or require Zed
to be installed. Keep the transport independent of the selected executable, but
negotiate capabilities rather than assuming all agents behave identically.
The [completed spike](12-codex-acp-spike.md) used a temporary client. The
[backend lifecycle](13-agent-run-lifecycle.md) now implements background runs,
controls, and explicit continuation. Its fresh-run path was manually validated
with Codex; the subsequent manual continuation check was also reported successful.
Automatic native snapshotting follows successful turns and passed the manual
real-Codex acceptance check. Explicit interrupted-run recovery is fake-agent/native-jj
tested; it never signals saved PIDs or automatically replays work.
Do not add a temporary one-shot CLI path.

## Session reuse

Reuse is optional per agent. Check capabilities at `initialize` (default is `loadSession: false`).

- `session/load`: restores context and replays history to the client (handle replay so it is not drawn as new output).
- `session/resume`: continues a session without replay; meant for agents that do not store full history.
- `session/list`: lists sessions the agent knows.
- `session/close`: cancels work and frees resources.
- `session/fork`: unstable; behind a feature flag in the Rust crate and an unstable capability in the spec.
- v2 direction: `list`, `resume`, `close` become required for agents that advertise sessions; replay folds into `resume`; fork stays optional.

The implemented `run continue` explicitly loads the same saved session in the
same workspace after checking capability support. It never substitutes a fresh
session on failure. Forking, resume without replay, and seeding another phase
from earlier context remain future design topics, not fallback behavior.

Caution to test: a spec passage suggests reloads work when the request's working directory matches the session's. With a separate jj workspace per phase, resuming from a different directory may not work.

## Diffs from the agent

Tool calls of kind `edit` can carry diff content (file path, old text, new text).
The Codex 2.5.0 spike observed these, including a replacement represented as
delete/add events. It also observed a proposed diff before permission was
granted: denying the request left the file absent. The later real backend run
created a file through a shell tool without a structured ACP edit diff. Do not
treat an event as proof of completion or expect every file change to have an
ACP diff. Native jj is authoritative. Other adapters remain unverified.

## Permissions

Use `session/request_permission` as the hook for approval gates. Combine with a separate jj workspace per phase so a bad run is easy to throw away.
