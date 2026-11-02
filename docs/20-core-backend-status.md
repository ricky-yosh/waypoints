# Core/backend status and documentation map

## How to read these docs

This page is the current capability inventory and the place to collect remaining
discussion items. It is **not a commitment to build every item or an ordered
roadmap**. Discuss scope and behavior before implementing a missing capability.

- [Architecture](02-architecture.md): ownership, boundaries, and enduring decisions.
- [Backend/CLI reference](11-backend-cli.md): current commands, RPC methods,
  transport, paths, timeouts, and validation commands.
- Feature guides linked below: exact behavior, safety rules, persistence, and
  feature-specific tests.
- [Build order and acceptance](07-spikes-and-build-order.md): integration checks
  and outstanding research, not a second implementation-status list.
- [Concept](01-concept.md) and broader design sketches describe the product
  direction; they do not establish that a feature is implemented.
- [Tool references](10-helpful-tools.md), [the ACP spike](12-codex-acp-spike.md),
  and [web research](08-proa-and-web.md) are research/history, not competing
  architecture decisions or pending implementation instructions.

When behavior changes, update this inventory, the backend reference when its
contract changes, and the relevant feature guide. Link to detailed rules rather
than copying them into every overview.

Use explicit architecture decisions for scope and feature guides for exact
implemented behavior; check the current source/tests when they disagree. A
historical test's "not exercised" list applies to that test, not necessarily
to later validation. No proposed finish line or backlog item becomes approved
work merely by appearing in a document.

## Implemented through core, backend, and CLI

| Capability | Current boundary | Detailed contract |
|---|---|---|
| Persistent workflows and prompt graphs | Canonical project-folder selection; multiple workflows; undoable renaming; no separate project registry | [Backend/CLI](11-backend-cli.md), [renaming](14-workflow-history.md#workflow-renaming) |
| Fixed workflow bases | Explicit local bookmark or revision, resolved once; omitting a selector still creates an unbased legacy workflow | [Workflow bases](16-workflow-bases.md) |
| Rooted node creation | New based-workflow nodes explicitly attach to the base or one or more upstream nodes; prompt creation does not create code | [Rooted nodes](17-rooted-node-dependencies.md) |
| Native association and change creation | Unique native changes; based nodes require matching parents; one or more upstream nodes can create clean or conflicted native merges | [Backend/CLI](11-backend-cli.md#associate-a-node-with-an-existing-jj-change), [combining work](17-rooted-node-dependencies.md#combining-parallel-work) |
| Native status refresh | Current/missing/divergent revisions and conflict flags, read-only; not a full ancestry/staleness assessment | [Native status](11-backend-cli.md#associate-a-node-with-an-existing-jj-change) |
| Read-only code review | Exact revision details, parent IDs, changed-file summary and Git patch for a node's current recorded commit or a run's saved snapshot; no snapshot/checkout mutation | [Code review](11-backend-cli.md#review-recorded-code-without-snapshotting) |
| Native revision history | Paginated native evolution, all divergent heads, predecessor links, saved-run annotations, and exact-commit previews; no workspace switching | [Revision history](22-native-revision-history.md) |
| Dedicated workspaces | Explicit on-demand allocation; status is read-only, verification may acknowledge database readiness but never changes native files/history; no automatic checkout repair or folder cleanup | [Workspaces](11-backend-cli.md#allocate-a-nodes-editing-workspace) |
| ACP runs and continuation | Trusted executable, verified workspace, bounded polling, explicit permissions/cancellation, fresh sessions or same-session follow-up | [Run lifecycle](13-agent-run-lifecycle.md) |
| Automatic successful-turn snapshots | Save an exact native revision and separate snapshot outcome; failure/cancellation preserves files without an automatic snapshot | [Revision recording](13-agent-run-lifecycle.md#automatic-revision-recording) |
| Ordinary leaf restoration | Restore a saved run or exact same-change commit, including manual revisions/checkpoints; preserve tracked edits first; fresh session required; descendants refused | [Restoration](21-saved-result-restoration.md) |
| Confirmed downstream restoration | Fresh equal preview and explicit confirmation; checkpoint allocated workspaces leaf-first; same-workflow native-aligned subtree rebasing; conflicts visible, descendant checkouts left stale, all old affected sessions barred | [Downstream restoration](24-downstream-restoration.md) |
| Readonly restore impact | Native/graph downstream tasks, merge/external work, workspaces, sessions and known blockers; no conflict simulation or downstream application | [Restore impact](23-restore-impact.md) |
| Informational upstream hint | Best-effort per-turn direct graph revisions; changed/unchanged/unknown comparison for users, never agent context or execution policy | [Upstream inputs](13-agent-run-lifecycle.md#informational-upstream-hint) |
| Interrupted-run recovery | Explicit stopped-writer attestation and fresh workspace checks; no process discovery, prompt replay, or lost-snapshot reconstruction | [Run recovery](13-agent-run-lifecycle.md#recovering-interrupted-runs) |
| Metadata undo/redo | Chronological per canonical project folder; workflow renaming, node creation/initial attachment, prompt edits, legacy connection creation, prompt-only move/delete; never code/conversation rewind | [Workflow history](14-workflow-history.md) |
| Prompt-only topology editing | Rooted based workflows; move a native-free subtree or delete one node and reconnect children; refuses native/workspace/run references throughout subtree | [Planning edits](14-workflow-history.md#prompt-only-move-and-delete) |
| Native reparenting | Move one native node and its checked subtree onto one replacement dependency in the same workflow | [Reparenting](18-native-reparenting.md) |
| Native deletion with retained folders | Abandon one native change, reconnect children, deregister its ready matching saved workspace while retaining every file and historical record; explicit resume for partial abandonment | [Deletion](19-native-node-deletion.md) |
| Interrupted dependency-operation recovery | Resolve clearly applied/unapplied reparent or delete without repeating jj; ambiguous states remain blocked | [Shared recovery](18-native-reparenting.md#explicit-recovery-for-clear-outcomes), [deletion recovery](19-native-node-deletion.md#shared-safety-and-recovery) |

The local backend runs on macOS/Linux, owns the database, and serializes requests
and metadata writes. It is selected by database/socket configuration, not
automatically one process per repository. One database can contain workflows for
multiple project folders. Clients never open the store directly.

### Important distinctions

- **Based versus legacy:** based graphs represent rooted native dependencies.
  Legacy unbased connections remain metadata-only; they are not silently migrated.
- **Saved reachability versus native state:** `connectivity.status: "rooted"` does not
  prove current native ancestry, fresh physical checkouts, or fresh agent context.
- **Session continuation versus code history:** `run continue` loads the latest
  eligible saved harness session; it does not select an old native commit.
  `run restore` or `node restore --commit` restores code and bars old-session
  continuation on that node.
- **Tool permission versus review approval:** implemented ACP permissions control
  offered tool requests. A completed run/saved snapshot is not human approval of
  the code or a workflow review gate.
- **Safe refusal versus complete repair:** native/metadata mismatch, external
  descendants, stale checkouts, or ambiguous interrupted operations can remain
  blocked. Refusal is intentional; general reconciliation is not implemented.
- **Recorded references versus retention:** stored native commit IDs and session
  IDs do not guarantee indefinite jj/harness retention.

## Remaining capabilities and decisions for discussion

These are documented gaps, **not newly approved work or a priority order**.

**V1 scope decision:** [reflect native jj changes, not inferred context staleness](02-architecture.md#decision-reflect-native-jj-changes-not-inferred-context-staleness).
Users infer changed assumptions from revisions and dependencies. A separate
downstream-staleness badge, acknowledgement/execution gate, or general agent-context
tracker is not required before the GUI or for v1 completion. Existing physical
checkout diagnostics, restore-specific session barriers and the informational CLI
upstream hint remain unchanged.

| Area | What is still missing or undecided |
|---|---|
| History UI and broader restoration | Visual revision picker; cross-workflow/unmanaged affected work and cross-change/changed-parent restoration beyond implemented leaf and checked same-workflow downstream restoration |
| Workspace folder cleanup | Optional filesystem cleanup beyond implemented deregistration; retained folders, ignored/untracked files, and interrupted removal policies |
| Editing combined branches | Changing an existing merge's parent set; richer per-input review and conflict-resolution policy beyond native conflict flags, materialized markers, and current node/run review |
| Broader reconciliation | External jj changes, native-associated legacy adoption, cross-workflow affected work, and ambiguous operation outcomes |
| Base and project/workflow management | Explicit base updates, whether new workflows should require/default a base, project registration, and broader workflow management |
| Review and execution policy | Human review gates distinct from tool permissions; context handoff between phases; additional agent configuration/capability support |
| Operational durability and portability | A documented backup/restore workflow, broader schema/history upgrade and downgrade policy beyond the targeted restore-journal migration, repository relocation/replacement, and whether portable workflow import/export is wanted |
| Control responsiveness | Whether serialized long native operations delaying cancellation/permission requests is acceptable, or requires a different scheduling design |
| GUI | Backend integration and product UI remain pending; GUI work must not introduce a separate workflow engine |

No MCP adapter, custom agent loop, transcript archive, or durable notification
replay is required by the current architecture. Polling is intentional, not a
missing subscription feature that must be added before using the CLI.

## Validation and known limits

Feature guides distinguish automated disposable-repository/fake-agent tests from
manual real-Codex acceptance. The full headless test/build commands live in
[backend checks](11-backend-cli.md#checks); individual passing tests do not prove
the complete product acceptance workflow.

The integrated branching/upstream-edit/restart/review scenario remains only
partially satisfied; see [acceptance checklist](07-spikes-and-build-order.md#headless-acceptance-checklist).
The narrower API/login preview → confirmed restore/rebase → manual checkout
update → explicit fresh session flow passed [real-Codex acceptance](24-downstream-restoration.md#manual-real-codex-acceptance),
including preserved edits, session barriers, exact reviews and backend restart.
That clean-rebase check does not establish real-agent conflict/crash recovery.
Do not substitute saved-session continuation for code restoration, or treat a
clean native rebase as proof of behavioral correctness. Inferring agent-context
validity is outside v1 scope, not a missing acceptance requirement.

Ignored `target/` evidence paths in feature guides identify local artifacts from
those checks; they are not shipped fixtures or guaranteed to exist in another
checkout. Turn-local editing workspaces cannot be reused after their scratch
directory expires. The guides preserve outcomes and limits independently.

An ownership-handoff startup failure is now covered by a deterministic regression:
the same pending request can start successfully when a previously unavailable
ownership lock is released. Clients retry only a starter's explicit lock-contention
exit status, within the original deadline; see [lifecycle](11-backend-cli.md#lifecycle).
This closes the reproduced handoff gap, not proof that every previously observed
intermittent restart failure had that cause. Serialized native requests can also
delay run controls; this is a documented limit, not a general concurrency guarantee.
