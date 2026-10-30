# Spikes, build order, open questions

## Spikes (small throwaway Rust programs, each under a day)

These are integration research topics, not an instruction to implement ACP first. Establish the headless core/backend/CLI foundation below first; ACP experiments can follow when adding harness execution.

1. **ACP basics.** Spawn Claude's adapter, send one prompt, log every event. Does it stream? Does it emit diff content for edits? Does it send permission requests you can answer?
2. **Session reuse.** Read capabilities from `initialize`. Try resume, load, and fork. Try resuming from a different working directory (matters for per-phase jj workspaces).
3. **jj workflow.** In a scratch repo: change per phase, two parallel workspaces, a merge, then edit an early change and observe rebase and conflicts. Confirm exact commands. Check whether Zed shows a phase's diff in a colocated repo.
4. **Turso.** Throwaway table, a few async writes, reopen the file. Confirms the crate works as expected; keep SQL plain.

Write down what you learn; it becomes the spec for the real core crate.

## Build order

**Current decision: core and CLI first, GUI later.** This supersedes the earlier requirement to include a node UI in the first usable slice.

1. Define the foundation: workflows, phases/prompts, relationships, and the meaning and lifecycle of a session. Keep Waypoints metadata separate from harness-owned conversations.
2. Implement the headless core and persistence: create and manage multiple workflows, with explicit identity and workflow selection. Introduce basic jj queries/associations without building a new code-history system.
3. Expose operations through the local JSON-RPC backend and thin CLI. Keep database access in the backend; support noninteractive commands, structured output, and clear errors.
4. Verify the headless workflow end to end: create two workflows, edit them independently, manage the agreed session metadata, and reopen after a backend restart. Confirm client reconnect/refresh and persistent workflow undo/redo.
5. Once the foundation works, add the GPUI client using the same backend operations. Verify CLI edits appear in the GUI before expanding node interactions.
6. Evaluate ACP execution integration separately; it is not a prerequisite for establishing workflows and session metadata.

No MCP adapter, transcript archive, or durable notification replay is required. See the [architecture decision](02-architecture.md#architecture-decision-local-backend-gui-and-cli).

## Open questions

- What does a Waypoints session represent: an active workflow context, a reference to a harness conversation, or another explicitly defined scope? Settle this before implementing session lifecycle.
- When an upstream phase is edited, should downstream phases auto re-run, or be marked stale for the user to choose?
- Does the Claude ACP adapter emit diff content for edits?
- Which agents in practice support resume, load, or fork, and does a session survive a change of working directory?
- Does Zed show a phase's diff correctly in a colocated jj repo?
- Name, and whether this is a private tool or something to publish.
- Where the author reviews agent-written code today (Zed, Delta, other), so "Open in" targets it first.

## Unverified claims to re-check

- Exact `jj` flags and revsets listed in `04-jj-snapshots.md`.
- Stability and package names of the Claude and Codex ACP adapters.
- Whether Zed's internal crates can be embedded.
- Whether Proa supports service workers or PWA behavior (see `08-proa-and-web.md`).
