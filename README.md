# Waypoints: plan-driven agent workflows

Status: early implementation. Persistent workflows, prompt nodes, branching
connections, and graph retrieval work through an automatically started backend
and CLI on macOS/Linux. The GUI and jj/agent integration are not connected yet.

## Try the CLI

From this repository, create a workflow for the current project:

```sh
cargo run -p waypoints-cli -- workflow create --project . --name "First workflow"
cargo run -p waypoints-cli -- workflow list --project . --json
```

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

Waypoints turns a plan into a graph of phases. Each phase runs in a coding agent over ACP (Claude, Codex, Gemini, and others), streams its work live, and is saved as a jj change. Review the diffs, branch parallel phases, edit an earlier step, and see exactly which downstream phases went stale. Built in Rust with GPUI.

## Files in this folder

| File | What it covers |
|---|---|
| `01-concept.md` | The problem, the core idea, what is deliberately not being built, naming |
| `02-architecture.md` | Layers, components, phase state machine, data to store |
| `03-acp-and-agents.md` | How ACP works, which agents support it, sessions, permissions, diffs |
| `04-jj-snapshots.md` | One jj change per phase, parallel phases, conflicts, stale tracking |
| `05-ui-and-diffs.md` | GPUI app layout, three levels of diff viewing, markdown |
| `06-stack-and-storage.md` | Rust, GPUI, Turso, React prototype, what to translate |
| `07-spikes-and-build-order.md` | Throwaway experiments to run first, build order, open questions |
| `08-proa-and-web.md` | What Proa is and where it could fit (optional web side) |
| `09-ideas-explored.md` | Other project ideas considered along the way, and why they were set aside |

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
