# Spikes, build order, open questions

## Spikes (small throwaway Rust programs, each under a day)

Run in this order; each answers one question that could change the design.

1. **ACP basics.** Spawn Claude's adapter, send one prompt, log every event. Does it stream? Does it emit diff content for edits? Does it send permission requests you can answer?
2. **Session reuse.** Read capabilities from `initialize`. Try resume, load, and fork. Try resuming from a different working directory (matters for per-phase jj workspaces).
3. **jj workflow.** In a scratch repo: change per phase, two parallel workspaces, a merge, then edit an early change and observe rebase and conflicts. Confirm exact commands. Check whether Zed shows a phase's diff in a colocated repo.
4. **Turso.** Throwaway table, a few async writes, reopen the file. Confirms the crate works as expected; keep SQL plain.

Write down what you learn; it becomes the spec for the real core crate.

## Build order

1. Core crate: plan and state tables, state machine.
2. One ACP agent running one phase with live streaming.
3. jj snapshot per phase.
4. Plan blocks UI.
5. Node canvas (last).

## Open questions

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
