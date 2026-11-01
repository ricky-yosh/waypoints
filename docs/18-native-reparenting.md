# Native node reparenting

[Native node deletion](19-native-node-deletion.md) uses this same durable
operation lifecycle and recovery. The shared CLI names are now
`node operation-recovery` and `node recover-operation`; the original
`reparent-recovery` / `recover-reparent` names remain supported aliases.

## Implemented behavior

`node reparent` moves an existing native node onto one replacement dependency:
another node in the same rooted workflow, or the workflow's exact saved base.
It uses `jj rebase --source SOURCE_COMMIT --destination DESTINATION_COMMIT`,
so the node's native descendants move with it. It does not execute agents.

```sh
# Use actual workflow/node IDs.
waypoints node reparent --workflow 1 --node 3 --parent 1 \
  --accept-undo-boundary --json

# Or attach directly to the fixed workflow base.
waypoints node reparent --workflow 1 --node 3 --from-base \
  --accept-undo-boundary --json

waypoints node operations --workflow 1 --json
waypoints workflow graph --workflow 1 --json
```

The source must already have a native change and exactly one saved dependency.
The replacement parent must have one current native change, unless selecting the
base. This is not a merge-parent editor or a prompt-only graph-move command.
Creating a prompt with its initial attachment remains metadata-only and undoable.
Legacy `connection create` does not acquire new rebase semantics.

Successful output includes exact before/after revisions, stable change IDs,
conflict flags, the previous and replacement parents, and a durable operation ID.
Conflicts are completed native outcomes, not a reason to roll back or rerun.

## Safety and scope

Before mutation, Waypoints checks:

- A based, explicitly rooted graph and valid same-workflow endpoints.
- No graph or native cycle, no unchanged dependency, and one existing parent.
- Current, unambiguous native associations and matching native parent commits
  throughout the selected workflow; no shared ownership of based changes.
- Mutability of every native revision in the affected subtree.
- Exact agreement between the graph's materialized subtree and native descendants.
  **External or other-workflow descendants are refused**, not silently rebased.
- No active/interrupted/finalizing run, incomplete workspace allocation, or
  uncertain change creation anywhere in this database.
- A stable native operation throughout preflight.

The broad database-wide writer check is deliberately conservative. Independent
databases and external jj processes are not locked by Waypoints. Do not run
external writers concurrently. The native command is pinned to the validated
operation; concurrent native operation heads are refused rather than merged.
Postflight requires a single native operation directly following the validated
one, then verifies actual native ancestry and stable subtree identities before
committing the replacement graph edge.

Read-only preflight has a 40-second overall deadline. Native mutation and
reconciliation together have a 70-second deadline (the jj mutation itself has
60 seconds). The RPC/client deadline is 120 seconds. Output remains bounded.
Backend requests are serialized, so other requests can wait during this command.

## Files and workspaces

The rebase always ignores working copies: it neither snapshots dirty files nor
updates any physical checkout. Dirty files, workspace folders, and checkout
metadata remain untouched. Affected workspaces can therefore become stale.
Existing run preflight refuses stale checkouts; Waypoints does not automatically
repair them or claim an agent can immediately continue.

Inspect and resolve checkout/conflict state using native jj before verifying a
workspace again. No directory cleanup, run-record rewrite, or conversation rewind
occurs.

## Undo boundary

`--accept-undo-boundary` is mandatory in effect, not a force option. Without it,
core/RPC reject the operation before any history or native mutation.

After all preflight checks pass, one database transaction:

1. Saves unresolved operation intent and exact native inputs.
2. Clears the canonical project's prior metadata undo **and redo** entries and
   resets its cursor. All workflows sharing that project use this new baseline.

The transaction commits **before** invoking jj. Rejected preflight or failed
reservation preserves history. Once reserved, even an unsuccessful or uncertain
native operation retains the boundary. This avoids leaving old undo actions
available after a crash. Native jj history is not deleted.

After successful reconciliation, new metadata edits are undoable up to this
baseline, never across it. Reparenting itself is not a metadata undo/redo action,
and Waypoints never invokes repository-wide `jj undo`.

## Interrupted or uncertain outcomes

jj and the workflow database cannot commit atomically. The additive
`dependency_operations` table keeps an unresolved reservation until both native
ancestry verification and graph persistence succeed. The final graph edit and
completed operation record are saved in one database transaction.

If the backend dies or any native/postflight/storage step fails:

- No automatic retry, rollback, reservation release, or startup repair occurs.
- The saved graph may still show the old edge while native code has moved.
  Graph JSON includes `unresolved_dependency_operation: OPERATION_ID`; human
  graph output explicitly warns that saved edges may not match jj.
- `node operations --workflow ID` retains exact inputs and any available error.
- Node edits, dependency adoption, metadata undo/redo, association/change/workspace
  creation, and new runs are blocked database-wide while the reservation remains.
  Read-only graph/native/history/operation inspection remains available.

### Explicit recovery for clear outcomes

First inspect the saved operation and read-only native assessment:

```sh
waypoints node operations --workflow 1 --json
waypoints node operation-recovery --operation 1 --json
```

The assessment is one of:

| Status | Meaning |
|---|---|
| `not_applied` | Native operation head and exact subtree revisions still match the preflight inputs; the old graph is consistent |
| `applied` | Exactly one operation followed preflight, and current ancestry, change identities, destination, and saved associations match the intended rebase |
| `blocked` | State is missing, divergent, changed, unavailable, or otherwise ambiguous; the response includes a reason |
| `resolved` | The saved operation is already completed or explicitly marked not applied |

Inspection never releases the reservation and **cannot prove old processes have
stopped**. Before requesting recovery, ensure the old native command and all other
repository writers have actually stopped. Waypoints does not signal stored PIDs.

```sh
waypoints node recover-operation --operation 1 \
  --confirm-processes-stopped --json
```

The confirmation is a caller attestation, not a force option. Core/RPC require it
too. Recovery rechecks current state rather than trusting an earlier inspection:

- `not_applied`: leave graph edges unchanged and mark the saved operation
  `not_applied`, releasing its reservation.
- `applied`: finish the verified graph edit and mark the operation `completed`,
  atomically with releasing its reservation. Native conflicts remain reported.
- `blocked`: return an error and keep the reservation. Do not retry the rebase,
  discard files, or edit database tables to force progress.

Both recovery paths only query jj: **no native mutation, snapshot, checkout update,
undo, or process launch to redo the rebase**. Dirty files, run records, workspace
folders, and the existing metadata undo boundary are preserved. Old undo/redo is
not resurrected even when the rebase never happened.

The operation record and graph are rechecked in the final database transaction.
A changed record or failed write rolls back all metadata changes and retains the
reservation. Retrying recovery of an already resolved operation returns that same
saved result without affecting a newer operation or its reservation.

Recovery checks have a 40-second overall deadline (120-second client/RPC
deadline). There is no startup auto-recovery, general repair, or reconciliation of
intervening external operations. Those ambiguous cases deliberately remain blocked.

## RPC and core

- `node.reparent {workflow_id, node_id, parent, acknowledgement?}`
  returns `DependencyOperation`.
- `node.operations {workflow_id}` returns saved operations in ID order.
- `node.operation_recovery {operation_id}` returns saved intent and a read-only
  `DependencyRecovery` assessment.
- `node.recover_operation {operation_id, confirmation?}` returns the resolved
  operation. Confirmation must be `"processes_stopped"`; omitted/null is refused,
  and booleans/other strings/unknown fields are invalid parameters.
- Parent: `{"kind":"workflow_base"}` or `{"kind":"node","node_id":1}`.
- Acknowledgement: `"accept_undo_boundary"`; omitted/null is refused before
  mutation, booleans/other strings/unknown fields are invalid parameters.

`node.reparent_recovery` and `node.recover_reparent` remain RPC aliases.

Core exposes the same operation and a typed `UndoBoundaryAcknowledgement`;
CLI-only parsing cannot bypass enforcement. Protocol version remains 1 with
additive methods/fields. Rebuild and stop an older backend before using them.

## Validation

Disposable native-jj tests cover a real file-bearing chain and descendants,
stable identities, inherited-code removal, conflicts, base and node destinations,
dirty-file/checkout preservation, new metadata undo after a boundary, and restart.
Refusal tests cover acknowledgement, cycles, wrong/no-op parents, external
descendants, immutable work, ancestry drift, and active/incomplete work.

Storage fault tests cover reservation failure without native/history mutation,
restart from the durable pre-native crash state, and actual native success
followed by graph-write failure without a partial metadata edge update. An
external native write between rebase and reconciliation blocks completion.
CLI/RPC tests cover acknowledgement enforcement, strict parameter shapes,
empty-change subtree rebasing, graph/undo results, and persisted inspection.
Real backend-crash tests stop fixture-owned backend/native processes before and
after the native mutation, then verify explicit recovery across restart without
repeating jj or recording dirty files. Core tests cover ambiguous external writes,
fresh checks after inspection, missing confirmation, preserved undo boundaries,
SQL-failure rollback, changed-record rejection, and idempotent old-operation retries
while a new reservation is active.
These do not claim GUI integration, real Codex execution, automatic workspace
repair, or multi-workflow reconciliation.
