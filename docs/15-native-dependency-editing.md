# Decision: workflows represent native code dependencies

## Status and precedence

**Agreed product direction; implementation in progress.** Explicit
[workflow base selection](16-workflow-bases.md) now works through backend/CLI.
[Rooted node creation](17-rooted-node-dependencies.md) now enforces explicit saved
dependencies in based workflows, including native creation/association checks and
explicit prompt-only legacy adoption.
[Native reparenting](18-native-reparenting.md) now rebases a single workflow's
checked subtree with durable intent, conflict reporting, and an undo boundary.
External/other-workflow descendants are refused. Explicit recovery handles clearly
applied or unapplied interrupted operations; ambiguous states remain blocked.
[Native node deletion](19-native-node-deletion.md) now abandons nodes and reconnects
children, deregistering a ready matching saved workspace while retaining its folder.
Explicit resume handles a verified partial abandonment. Folder cleanup remains
unimplemented.

This decision supersedes the earlier proposal to make metadata-only removal the
normal deletion behavior for native-backed nodes. Prompt-only nodes now have
[undoable metadata move/delete](14-workflow-history.md#prompt-only-move-and-delete).
It also supersedes the broader design in which
connections express only planning intent. Legacy unbased connection commands
retain that metadata-only model; based workflows use explicit rooted dependencies
and checked native edits. Do not silently reinterpret legacy commands or migrate
saved graphs by guessing native ancestry. See the
[status map](20-core-backend-status.md) for the current capability boundaries.

## User-facing rules

- A based workflow has an explicit native base, represented as a permanent graph root,
  not a deletable prompt node. Select an exact starting revision, not an
  unannounced moving `main`.
- Every saved dependency node must have a path to that base. A node may attach
  directly to the base; it does not have to depend on another prompt node.
- In based workflows, connections represent code dependencies. Creating a
  prompt and its initial attachments is metadata-only; creating its native
  change uses those dependencies. Editing existing native dependencies uses
  validated jj operations, never a metadata-only edge replacement.
- Do not save orphaned intermediate states. Reject removal of the last upstream
  dependency unless a replacement, such as the workflow base, is explicitly
  included in the same operation.
- Deleting a node with native work abandons its associated jj change. jj rebases
  its children onto its parents; Waypoints reflects the resulting ancestry.
  Do not leave descendants disconnected or automatically execute them.
- Native conflicts are outcomes to expose, not evidence that abandonment failed
  or permission to discard child work.
- Deregister a deleted node's ready, matching Waypoints-created workspace while
  retaining its complete folder. Dirty/ignored/untracked files remain untouched,
  not discarded. Refuse active/unresolved writers and unsafe allocations;
  workspace allocation is on demand, so some nodes have none to deregister.
- Retain run/snapshot records as historical references, not active graph nodes.
  They do not constitute a transcript archive or pin native revisions forever.
- Code-changing actions require explicit undo-boundary acknowledgement. Workflow
  Undo neither crosses that boundary nor invokes repository-wide `jj undo`.
  Native code recovery is separate; agent conversation memory never rewinds.

Deleting a native node is therefore not an undoable metadata-only archive action
followed by a separate optional discard. Do not introduce that earlier proposal
as the default behavior.

Example:

```text
Before: Base → Interfaces → Authentication → Review
After:  Base → Interfaces → Review
```

Review retains its own edits but no longer inherits Authentication's contribution.
It may become conflicted if those edits depended on Authentication.

## Acknowledgement

Use the [shared undo-boundary policy](14-workflow-history.md#decision-acknowledge-future-code-changing-connections)
for code-changing connections and node deletion. The implemented reparent CLI flag is
`--accept-undo-boundary`, with an inline explanation and no interactive prompt.
RPC/core must require equivalent explicit acknowledgement. Missing acknowledgement
must fail without mutation; acknowledgement does not bypass validation or authorize
deleting unrecorded files.

## Native validation: jj 0.46.0

Tests in `crates/waypoints-core/tests/jj.rs` execute
`jj --ignore-working-copy --at-op=@ abandon CHANGE_ID` in disposable repositories.
They inspect recorded state without a later snapshot/checkout masking effects.
They validate default abandonment, **not** `--restore-descendants`:

| Case | Observed result |
|---|---|
| Middle of a chain, independent file edits | Child keeps its change ID, gets a new commit ID and the abandoned change's parent; abandoned contribution disappears from its recorded tree |
| Fan-out | All native children reconnect, including a child representing work outside the selected workflow |
| Abandon a two-parent merge | Its child acquires both native parents |
| Abandon one parent of a merge | The merge keeps the other parent and inherits the abandoned parent's parent, even when that inherited parent is also an ancestor of the other parent |
| Child modifies a file introduced by the abandoned change | Abandonment succeeds and the child records a conflict, retaining its change identity |
| Another workspace edits the abandoned change | jj assigns that workspace a replacement change; its directory, dirty file, and checkout metadata remain untouched by the ignore-working-copy operation |

The last case is why repository mutation and workspace deregistration need
separate tracked steps. Native abandonment is neither a folder-deletion command
nor proof that affected physical checkouts are current. These low-level tests
are distinct from the subsequent [product-path deregistration tests](19-native-node-deletion.md#validation).
Filesystem removal remains unimplemented.

## Implemented lifecycle and remaining scope

The backend/CLI implements rooted creation, checked same-workflow native edits,
durable intent, clear-outcome recovery, and saved-workspace deregistration with
all files retained:

1. Preflight exact saved/native ancestry, ownership, mutability, allocations,
   writers and a stable native operation. Refuse unmanaged/other-workflow native
   descendants rather than silently mutating them.
2. Require explicit undo-boundary acknowledgement for reparent/delete. Persist
   intent and clear the project's old metadata history atomically before jj runs.
3. Run the exact validated native operation and verify actual resulting
   identities, parents and conflicts. For deletion, separately checkpoint
   abandonment before deregistering a saved workspace.
4. Commit verified graph changes and completion together. Uncertainty retains
   the reservation; do not automatically retry or claim native rollback.
5. Expose inspection and explicit stopped-writer recovery. A verified partial
   deletion may explicitly resume only the remaining deregistration.

Cross-workflow/unmanaged reconciliation, editing an existing merge's parent
set, native-backed legacy adoption, filesystem cleanup and general ambiguous-state
repair remain separate discussion items, not requirements implied by this lifecycle.

Metadata records and native ancestry remain distinct stores, but must not be
presented as unrelated graphs the user has to keep aligned manually. External jj
editing remains allowed. Native refresh reports revisions/missing/divergent/conflict
state; it is not full ancestry reconciliation. Checked operation preflight
validates ancestry and refuses mismatches rather than guessing associations or
silently rewriting code.
