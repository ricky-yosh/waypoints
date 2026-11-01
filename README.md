# Waypoints: plan-driven agent workflows

Status: early headless implementation on macOS/Linux. Core/backend/CLI support
persistent rooted workflows, native jj changes and on-demand workspaces, ACP runs
and continuation, automatic successful-turn snapshots, same-change saved-result restoration,
metadata undo/redo, native
reparenting, multi-parent native creation, native deletion with retained workspace folders,
read-only native code review and paginated revision history,
an informational upstream-revision hint, and undoable prompt-only move/delete.
Explicit recovery handles clear interrupted outcomes; ambiguous states remain
blocked. GUI integration and downstream context-staleness
tracking remain pending.

Start with the [current core/backend status and documentation map](docs/20-core-backend-status.md)
for implemented boundaries, remaining discussion items, and links to detailed contracts.

## Try the CLI

For an existing jj project with a local `main` bookmark, explicitly save its base:

```sh
cargo run -p waypoints-cli -- workflow create --project . --name "First workflow" --base main
cargo run -p waypoints-cli -- workflow list --project . --json
```

Choose another bookmark or `--base-revision` when appropriate. Omitting both
selectors still creates a legacy unbased workflow, not a workflow implicitly
rooted at `main`; see [base compatibility](docs/16-workflow-bases.md#compatibility-and-existing-workflows).

The first workflow command starts the local backend automatically. Later commands
reuse it; the CLI never opens the database directly.

```sh
cargo run -p waypoints-cli -- backend status --json
cargo run -p waypoints-cli -- backend stop
```

Stopping the backend preserves saved workflows. The next workflow command starts
it again. See [backend and CLI usage](docs/11-backend-cli.md) for paths, isolated
test data, the local protocol, and current limitations.

## One line

A desktop app for planning and running AI coding work as phases: each phase gets its own agent session and a jj snapshot, and you review the code as it's written.

## Short description

Product direction, not a list of implemented GUI/backend capabilities:

Waypoints turns a plan into a graph of phases. Each phase runs in a coding agent over ACP (Claude, Codex, Gemini, and others), streams its work live, and is saved as a jj change. Review the diffs, branch parallel phases, edit an earlier step, and see exactly which downstream phases went stale. Built in Rust with GPUI.

## Documentation

| File | What it covers |
|---|---|
| [`20-core-backend-status.md`](docs/20-core-backend-status.md) | Start here: current capability inventory, documentation map, and unprioritized discussion backlog |
| `01-concept.md` | The problem, the core idea, what is deliberately not being built, naming |
| `02-architecture.md` | Layers, components, phase state machine, data to store |
| `03-acp-and-agents.md` | How ACP works, which agents support it, sessions, permissions, diffs |
| `04-jj-snapshots.md` | One jj change per phase, parallel phases, conflicts, stale tracking |
| `05-ui-and-diffs.md` | GPUI app layout, three levels of diff viewing, markdown |
| `06-stack-and-storage.md` | Rust, GPUI, Turso, React prototype, what to translate |
| `07-spikes-and-build-order.md` | Throwaway experiments to run first, build order, open questions |
| `08-proa-and-web.md` | What Proa is and where it could fit (optional web side) |
| `09-ideas-explored.md` | Other project ideas considered along the way, and why they were set aside |
| [`11-backend-cli.md`](docs/11-backend-cli.md) | Current CLI/RPC reference, transport, paths, lifecycle, limits, and validation commands |
| [`12-codex-acp-spike.md`](docs/12-codex-acp-spike.md) | Tested Codex ACP behavior, setup, caveats, and the next production slice |
| [`13-agent-run-lifecycle.md`](docs/13-agent-run-lifecycle.md) | Implemented background-run checkpoint, commands, bounds, cleanup, and remaining work |
| [`14-workflow-history.md`](docs/14-workflow-history.md) | Persistent metadata undo/redo, CLI/RPC, native-history boundary, and validation |
| [`15-native-dependency-editing.md`](docs/15-native-dependency-editing.md) | Agreed rooted code-dependency model, native abandonment tests, and implementation sequence |
| [`16-workflow-bases.md`](docs/16-workflow-bases.md) | Explicit bookmark/revision starting points, persistence, CLI/RPC, and legacy compatibility |
| [`17-rooted-node-dependencies.md`](docs/17-rooted-node-dependencies.md) | Explicit node attachments, rooted connectivity, native creation checks, and legacy adoption |
| [`18-native-reparenting.md`](docs/18-native-reparenting.md) | Checked native reparenting, code-changing undo boundaries, and unresolved operation safeguards |
| [`19-native-node-deletion.md`](docs/19-native-node-deletion.md) | Native node abandonment, child reconnection, workspace restrictions, and shared recovery |

## Decisions so far

| Topic | Decision |
|---|---|
| Agent integration | ACP client, one adapter per agent, no custom agent loop |
| Snapshots | One jj change per phase, parallel phases in separate jj workspaces |
| Diff viewing | Inline summaries in app, "Open in Zed" for deep review, no custom multibuffer |
| UI | GPUI with `gpui-component` |
| Storage | Turso (Rust SQLite rewrite, beta) with plain SQL so SQLite is a fallback |
| Prototype | React + TypeScript + Tailwind + React Flow, then translate decisions (not code) to GPUI |
| Web / mobile | Optional, undecided; Proa is a candidate for a web side |
