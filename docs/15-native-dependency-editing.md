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

- A workflow has an explicit native base, represented as a permanent graph root,
  not a deletable prompt node. Select an exact starting revision, not an
  unannounced moving `main`.
- Every saved dependency node must have a path to that base. A node may attach
  directly to the base; it does not have to depend on another prompt node.
- Connections represent native code dependencies. Creating/replacing one uses
  validated native jj operations, not only a metadata edge.
- Do not save orphaned intermediate states. Reject removal of the last upstream
  dependency unless a replacement, such as the workflow base, is explicitly
  included in the same operation.
- Deleting a node with native work abandons its associated jj change. jj rebases
  its children onto its parents; Waypoints reflects the resulting ancestry.
  Do not leave descendants disconnected or automatically execute them.
- Native conflicts are outcomes to expose, not evidence that abandonment failed
  or permission to discard child work.
- Clean up the deleted node's Waypoints-created workspace when safe. Refuse
  deletion if active/unresolved writers or unrecorded files would be put at risk.
  Workspace allocation is on demand, so some nodes have no folder to clean up.
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

The last case is why repository mutation and safe workspace cleanup need separate
tracked steps. Native abandonment is neither a folder-deletion command nor proof
that affected physical checkouts are current. The tests do not yet validate a
complete workspace-forget/filesystem-removal policy.

## Design sequence and remaining scope

This is the broader design sequence, not a list of entirely unimplemented work.
The current slices implement rooted creation, single-workflow checked native
edits, durable intent, and clear-outcome recovery. External/other-workflow
descendants are refused; cross-workflow reconciliation and safe workspace cleanup
below remain future design/implementation work.

1. **Rooted workflow foundation:** persist an explicit base; validate reachability
   and native associations; provide inspection and explicit adoption for legacy
   disconnected graphs. Preserve old data and reject ambiguous mappings.
2. **Native operation lifecycle:** preflight the whole affected native subtree,
   including other workflows and registered workspaces; reject unsafe writers,
   missing/divergent/immutable targets, unexpected ancestry, and shared associations
   that cannot be safely updated. No `--ignore-immutable` escape hatch.
3. **Durable operation intent:** reserve affected Waypoints work before native
   mutation; record exact inputs and operation progress. jj and Turso cannot commit
   atomically. A crash or uncertain native result retains a visible unresolved
   operation and blocks conflicting work; do not auto-retry or claim rollback.
4. **Delete and reconcile:** abandon the exact validated target, query actual
   resulting parents/revisions/conflicts, and transactionally update the active
   graph and its undo boundary. Handle affected nodes across workflows, not only
   the currently selected graph. Stop on external changes rather than overwriting.
5. **Workspace cleanup:** verify saved ownership/path and recorded/physical state,
   account for jj's replacement working-copy change, and track native deregistration
   and filesystem cleanup. Never infer safety just from an empty `jj diff`;
   ignored/untracked files matter. Interrupted cleanup must be inspectable.
6. **Expose through backend/CLI:** explicit acknowledgement, structured outcomes,
   and tests for refusals and crashes between stages. Only then add GUI actions.

Metadata records and native ancestry remain distinct stores, but must not be
presented as unrelated graphs the user has to keep aligned manually. External jj
editing remains allowed: detect mismatches on refresh/preflight and require
reconciliation rather than guessing associations or silently rewriting code.
