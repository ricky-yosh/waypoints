# Spikes, build order, open questions

The [core/backend status map](20-core-backend-status.md) owns the current capability
inventory and unprioritized discussion backlog. This page keeps integration
acceptance and research separate from implementation status.

## Spikes (small throwaway integration programs, each under a day)

These began as integration research topics, not an instruction to implement ACP
first. Core/backend/CLI and ACP execution now exist; the entries below distinguish
completed research from validation still needed.

1. **ACP basics — Codex spike completed.** Streaming, structured diffs, permission denial/approve-once, and protocol cancellation passed with adapter 2.5.0 using a temporary Python client. The subsequent [backend lifecycle](13-agent-run-lifecycle.md) is fake-peer tested, and its Rust fresh-run path also passed a manual Codex file-edit test. That run used a shell tool without a structured edit diff. See [spike results and integration rules](12-codex-acp-spike.md).
2. **Session reuse — partially verified.** Same-session follow-up and `session/load` after an adapter restart passed with Codex in the temporary spike, including history replay and conversation-only recall. An unprompted session failed to reload. Rust explicit continuation and automatic post-turn snapshotting also passed manual Codex checks. Interrupted-run recovery is now fake-peer/native-jj tested, preserving partial work without replay. Resume without replay, fork, and different-working-directory loading remain untested.
3. **jj workflow — partially verified.** Changes, separate workspaces, rewrites,
   abandonment, conflicts, and interrupted-operation recovery have executable
   coverage, including read-only current-node/saved-run diffs and native multi-parent
   creation with fake-agent execution, plus paginated native evolution and exact
   previews. Editing merge-parent sets, a visual revision picker, and the
   complete upstream-edit acceptance workflow remain incomplete. Editor handoff
   behavior still needs validation.
4. **Turso — persistence foundation implemented.** Reopen, additive upgrades,
   transactional metadata history, and injected SQL-failure tests exist. Backup/
   restore procedures and non-additive migration policy remain discussion items.

Write down what you learn; it becomes the spec for the real core crate.

## Build order

**Current decision: core and CLI first, GUI later.** This supersedes the earlier requirement to include a node UI in the first usable slice.

The shared backend remains the workflow engine. The GUI must use the same API,
not implement its own native editing, storage, or execution lifecycle.
Based workflows follow the [native dependency decision](15-native-dependency-editing.md);
legacy metadata-only commands retain compatibility rather than silently rebasing.

Do not infer the next feature from the order of historical spike entries.
Missing capabilities and their intended behavior need discussion before selecting
another implementation slice; see [discussion items](20-core-backend-status.md#remaining-capabilities-and-decisions-for-discussion).

## Headless acceptance checklist

The following integrated scenario is **partially satisfied**, not a claim that
every step is available today. Individual feature tests do not replace it.

- Create two workflows and edit them independently through the backend/CLI.
- In a based workflow, create interfaces/placeholders with authentication,
  image-uploading, and settings branches.
- Allocate separate workspaces for parallel work; run and explicitly continue an
  agent session, record exact successful-turn revisions, and inspect outcomes.
- Reparent/delete eligible native nodes and exercise explicit recovery in
  disposable crash scenarios; confirm graph/native alignment and undo boundaries.
- Reopen after backend restart; verify reconnect/refresh and persistent metadata
  undo/redo without code or conversation rewind.
- Inspect native diffs for current node revisions and exact saved run snapshots.
- Browse [native revision history](22-native-revision-history.md) through the
  CLI/backend. Restore eligible saved results within the same leaf change;
  this passed [manual Codex acceptance](21-saved-result-restoration.md#manual-codex-acceptance).
  Exact same-change manual revisions/checkpoints can also be restored by commit
  ID. [Readonly restore impact](23-restore-impact.md) now lists affected native/graph
  work, workspaces and sessions; it does not simulate conflicts or apply downstream
  changes. [Confirmed same-workflow application](24-downstream-restoration.md)
  now checkpoints edits and rebases checked descendants, with explicit fresh
  sessions and manual stale-checkout reconciliation. A visual picker and broader
  affected-work reconciliation remain incomplete.
  Same-session `run continue` is not code restoration.
- **Not yet complete:** edit upstream work and surface native ancestry mismatch,
  stale physical checkouts, native conflicts, and stale agent context distinctly.
- Combine branch results through native multi-parent creation, with clean/conflicted
  two-/three-parent tests and fake-agent snapshot/review coverage. A disposable
  two-parent clean merge also passed [manual Codex acceptance](17-rooted-node-dependencies.md#manual-codex-acceptance).
  Real-agent three-parent/conflicted integration and editing existing merge-parent
  sets remain separate work.

GUI integration remains pending. Its acceptance must include CLI edits appearing
in the GUI through backend refresh, not a second local store or workflow engine.

No MCP adapter, transcript archive, or durable notification replay is required. See the [architecture decision](02-architecture.md#architecture-decision-local-backend-gui-and-cli).

## Open questions

- How should the visual picker, downstream restoration and native retention limits be exposed beyond implemented browsing and same-change run/commit restoration?
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
