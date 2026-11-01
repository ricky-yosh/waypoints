# Native node deletion with retained workspace folders

## Implemented scope

`node delete` abandons one node's native jj change and removes that node from the
active workflow graph. Its children inherit its existing parent and retain their
own native changes. Further descendants are rebased by jj, not executed again.

```text
Before: Base → Interfaces → Authentication → Review
After:  Base → Interfaces → Review
```

Review no longer inherits Authentication's contribution. Dependent edits can
become conflicted; conflict flags are included in the completed operation.

```sh
waypoints node delete --workflow 1 --node 2 --accept-undo-boundary --json
waypoints workflow graph --workflow 1 --json
waypoints node operations --workflow 1 --json
```

Use actual workflow/node IDs. The workflow must be based and rooted; the target
must have one existing dependency and one current mutable native change. The
workflow base is not a deletable prompt node. Prompt-only deletion uses the separate
undoable `node delete-prompt` command. Deletion of a multi-parent native target is
not implemented in this slice.

Root-node deletion attaches its children to the fixed workflow base. Leaf
deletion removes just the leaf. Deleting the last node leaves an empty rooted
workflow. Fan-out is supported. Existing inherited child links are deduplicated;
actual native parent commits must match the proposed graph before it is saved.

## Workspace deregistration without folder deletion

A target may have one **ready, matching, Waypoints-saved workspace**. Deletion
explicitly abandons the source change first, verifies and checkpoints that native
operation, then runs pinned `jj workspace forget -- SAVED_NAME` from the project
repository. Finally it verifies the native graph and commits the metadata deletion.
Abandonment comes first because forgetting an empty, undescribed working-copy
change can itself abandon that change. jj may also discard the temporary empty
replacement working-copy change when the workspace is forgotten; it must not be
associated with a saved node.

The folder is **left on disk in full**: tracked modifications, ignored files,
untracked files, and local `.jj` checkout metadata are not snapshotted, updated,
or removed. The completed operation reports its retained path. It is no longer
a registered jj workspace; the retained `.jj` files are historical local metadata,
not a promise that normal jj editing commands in that folder still work.

Incomplete, missing-directory, stale, switched, symlinked, or otherwise mismatched
allocations are refused before the undo boundary. Unsaved/additional workspaces
editing the source change, including the original project checkout, are refused.
The project's repository workspace is never deregistered. No automatic repair,
workspace re-registration, directory removal, or file cleanup is provided.

Idle descendants can have workspaces. Their physical checkouts remain untouched
and may become stale after native rebasing; run preflight must verify them before
further execution. Do not treat a completed deletion as checkout repair.

## Shared safety and recovery

Deletion uses the same durable lifecycle as [native reparenting](18-native-reparenting.md):

- Require explicit undo-boundary acknowledgement in CLI, RPC, and core.
- Validate native ancestry and exclusive change ownership throughout the selected
  workflow; refuse missing/divergent/immutable work or an unrooted graph.
- Refuse external/other-workflow native descendants rather than silently changing
  work outside the checked graph.
- Refuse active/interrupted/finalizing runs and incomplete native allocations
  anywhere in this database.
- Persist intent and clear the project's prior metadata undo **and redo** in one
  transaction before mutation.
- Pin `jj --ignore-working-copy --at-op=EXACT_OPERATION abandon -- EXACT_COMMIT`
  to the validated operation. Never snapshot or update physical checkouts.
- For a saved workspace, verify and save the abandonment checkpoint before the
  separately pinned workspace-forget command. Never infer a name from the current directory.
- Verify that the target is no longer visible, all surviving change identities
  remain, and actual native parent commits match the reconnected graph.
- Save the graph changes, deletion marker, and completed operation atomically.
  A native conflict is a completed outcome, not a reason to repeat abandonment.

Preflight has a 40-second deadline; mutation and reconciliation together have
70 seconds, with a 60-second native command deadline. Client/RPC timeout is
120 seconds. Existing output bounds still apply.

An interrupted or uncertain deletion keeps the global reservation and the old
graph, visibly marked with `unresolved_dependency_operation`. Inspect and
explicitly recover it:

```sh
waypoints node operation-recovery --operation 1 --json

# Only after ensuring old native commands and all other writers have stopped:
waypoints node recover-operation --operation 1 \
  --confirm-processes-stopped --json
```

- Clearly unapplied: keep the node/edges and release the reservation, recording
  `not_applied`. Do not restore the old metadata undo/redo history.
- Clearly applied: finish the verified deletion metadata and release the
  reservation, recording `completed`.
- `deletion_abandoned`: the source was abandoned and its descendant graph was
  verified, but the saved workspace remains registered. Metadata-only recovery
  refuses this partial state. Explicitly authorize the remaining deregistration:

  ```sh
  waypoints node resume-deletion --operation 1 --confirm-processes-stopped --json
  ```

  Resume rechecks the exact native checkpoint, saved allocation, registry,
  descendants, and replacement working-copy ownership, then forgets the saved
  workspace and commits deletion. It never repeats source abandonment.
- Ambiguous or changed native/saved state: remain blocked with a reason.

Recovery never reruns abandonment, rewinds native code, snapshots files, signals
saved PIDs, or repairs a checkout. Repeating recovery of a resolved operation is
an idempotent no-op and cannot release a newer reservation.
Resume also treats resolved operations as idempotent no-ops. No resume or recovery
is automatic at startup. If the backend dies before saving the abandonment
checkpoint, inspection can recognize the exact single-operation abandonment and
resume can persist that checkpoint before continuing. If forgetting already ran,
ordinary metadata recovery completes without repeating either native command.
Intervening external native writes or changed allocation records remain blocked.

The former `node reparent-recovery` / `node recover-reparent` CLI names remain
aliases for the shared operation recovery commands.

## Historical data and persistence

Deletion is a real native abandonment, **not** metadata-only archiving.
The additive `deleted_nodes(node_id, operation_id)` table marks removal from active
graph/node APIs while retaining the original node row and native association for
historical foreign-key references. Incident edges and the base attachment are
removed; new inherited child attachments are saved in the same transaction.

Deleted nodes cannot be edited, associated, allocated a workspace, used as new
parents, or run again through active-node APIs. IDs stay reserved. Captured run
prompts, sessions, snapshot outcomes, and exact recorded revisions are not rewritten
or deleted. Historical `run list --workflow ID --node ID` and `run status --run ID`
remain readable, and `run review` still queries its exact saved snapshot. The
original workspace allocation is retained for history/path ownership, not reused
as a fresh allocation. This does not pin native revisions against jj garbage collection
or archive harness transcripts.

Operation records include `kind: "delete"` (or `"reparent"`). Old records without
`kind` deserialize as reparent operations. For deletion, `parent` and `destination`
describe the inherited parent; `before` includes the abandoned node and its native
descendants, while `after` contains only surviving descendants and conflict flags.
Workspace-backed records add `deletion_workspace: {workspace, abandoned_operation}`.
The workspace has its original repository/change/name/directory fields;
`abandoned_operation` is null before the checkpoint. Old records without
`deletion_workspace` retain the original no-workspace lifecycle.

## RPC

| Method | Parameters | Result |
|---|---|---|
| `node.delete` | `{workflow_id, node_id, acknowledgement?}` | `DependencyOperation` |
| `node.operation_recovery` | `{operation_id}` | `DependencyRecovery` |
| `node.recover_operation` | `{operation_id, confirmation?}` | `DependencyOperation` |
| `node.resume_deletion` | `{operation_id, confirmation?}` | `DependencyOperation`; explicitly permits remaining workspace deregistration |

Acknowledgement must be `"accept_undo_boundary"`; recovery confirmation must be
`"processes_stopped"`. Omitted/null is a domain refusal before mutation. Booleans,
wrong strings, unknown fields, and force flags are invalid parameters.
Old `node.reparent_recovery` / `node.recover_reparent` RPC names remain aliases.
Protocol version remains 1 with additive methods/fields. Rebuild and stop an older
backend before using them.

## Validation

Disposable native repositories and CLI/RPC tests cover file-bearing chains,
fan-out, root/leaf/last-node deletion, inherited-code removal, stable descendant
identities, native conflicts, read-only dirty-file/checkout preservation, restart,
consent and workspace refusals, external descendants, and deleted-node API guards.
Core tests retain historical run/snapshot data and exercise native-success /
SQL-failure rollback of edges, deletion marker, and reservation release.

Real backend-crash tests cover both sides of abandonment, followed by explicit
recovery without a second native mutation. Recovery tests also cover changed
native operations, missing confirmation, and old-record compatibility. Workspace
tests cover all four crash windows (before/after abandonment and forgetting),
including real backend/native-process kills, checkpoint-write failure, final
SQL rollback, retained dirty/ignored/untracked files and checkout bytes, historical
run review, explicit resume, and idempotent resolved retries.
No GUI integration, live Codex execution, folder cleanup, general
ambiguous-state repair, or multi-workflow reconciliation is claimed.
