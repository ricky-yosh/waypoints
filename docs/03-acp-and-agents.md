# ACP and agents

This page explains the protocol and earlier adapter research. The
[run lifecycle](13-agent-run-lifecycle.md) is the authoritative contract for
implemented behavior; protocol capabilities below are not all Waypoints features.

## Which side is the tool?

The Waypoints backend is the ACP **client** (the role editors like Zed play).
It launches an agent as a subprocess and talks JSON-RPC 2.0 over stdin/stdout.
Neither the CLI nor the desktop GUI hosts a separate agent loop.

In the [intended agent-led workflow](28-agent-led-workflows.md), the user talks
to a coordinator in their existing harness. That coordinator calls Waypoints'
CLI/tools to create workflows and manage work; the backend launches and controls
the worker agents over ACP. The coordinator and ACP workers have distinct roles.
Opening the visual does not itself start another worker or transfer the primary
conversation into the app.

## Basic message flow

- `initialize` negotiates version and capabilities. Authentication belongs to
  the agent; Waypoints currently requires existing authentication.
- `session/new` (with a working directory) or a way to reuse a session.
- `session/prompt` to send a prompt; the agent streams `session/update` notifications (message chunks, thought chunks, tool calls and updates, plans, mode changes).
- The agent may request `session/request_permission`. The protocol also permits
  client file/terminal callbacks, but Waypoints does not advertise or implement them.
- `session/cancel` interrupts; the turn ends with a stop reason on the `session/prompt` response.

## Why ACP over headless CLI

ACP was selected for typed streaming, explicit tool-permission callbacks,
cancellation, and capability-negotiated session loading. Waypoints does not add
a second one-shot agent CLI path. Its own headless CLI controls the ACP-backed
backend; it is not an alternative agent harness. Tool permissions are not human
code-review gates.

## Agent support

Earlier registry research found the following candidates. This is not a tested
Waypoints support matrix or a promise about current package names, flags, or
capabilities; verify them before adding another adapter.

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
controls, and explicit continuation. Fresh sessions, same-session follow-up,
automatic snapshots, restoration and explicit fresh execution after restoration
have [manual real-Codex acceptance](24-downstream-restoration.md#manual-real-codex-acceptance).
Explicit interrupted-run recovery is fake-agent/native-jj
tested; it never signals saved PIDs or automatically replays work.
Do not add a temporary one-shot CLI path.

## Session reuse

Reuse is optional per agent. Check capabilities at `initialize` (default is
`loadSession: false`). The following are protocol/research concepts, not a list
of implemented Waypoints commands:

- `session/load`: restores context and replays history to the client (handle replay so it is not drawn as new output).
- `session/resume`: continues a session without replay; meant for agents that do not store full history.
- `session/list`: lists sessions the agent knows.
- `session/close`: cancels work and frees resources.
- `session/fork`: unstable; behind a feature flag in the Rust crate and an unstable capability in the spec.
- Proposed protocol changes must be checked against the selected version;
  Waypoints currently negotiates ACP version 1.

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

Waypoints exposes offered single-use tool decisions through
`session/request_permission`; it never grants blanket approval implicitly.
Workspaces isolate simultaneous code editing, not the agent's OS privileges.
Permission enforcement depends on the trusted executable. Denial/cancellation
does not discard partial files, and deleting a native node retains its folder.
Human code-review approval is a separate, unimplemented capability.
