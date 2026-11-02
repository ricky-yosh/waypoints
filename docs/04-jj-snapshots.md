# jj snapshots per phase

See [core/backend status](20-core-backend-status.md) for the capability inventory.
This page explains native change/revision semantics and links to implemented contracts.

## Implemented slice

The CLI can explicitly associate a saved node with an existing jj change and
refresh its current revision, description, and conflict status. Missing changes
retain their saved nodes; divergence is surfaced without choosing a revision.
Queries do not snapshot dirty working copies or mutate native history. See
[backend and CLI usage](11-backend-cli.md#associate-a-node-with-an-existing-jj-change).
An unassociated node can also explicitly create an empty jj change without moving
or snapshotting the working copy. Based workflows derive the starting revision
from the saved dependency; legacy unbased workflows require an explicit base.
Existing associations, including missing ones,
are refused; uncertain creation outcomes require manual inspection and association.
Nodes can allocate dedicated native workspaces on demand, at explicit new
directories. Allocation edits the node's existing change, not a new child; it
refuses changes already edited by another registered workspace. Workspace status
and repeat allocation requests are read-only, with missing, mismatched, and
incomplete references retained instead of silently replaced.
Explicit workspace verification can acknowledge a completed checkout by changing
only the saved readiness flag; it never repairs native state or overwrites edits.
Successful agent turns now automatically snapshot their dedicated workspace and
record an exact revision before releasing the run reservation. Native snapshot
failures are distinct from agent failures; cancelled/failed turns are not
automatically snapshotted. See [run finalization](13-agent-run-lifecycle.md#automatic-revision-recording).
The visual history picker and workflow-level checkpoints are not implemented.
General inferred-context-staleness tracking is outside v1 scope.

## Model

The [rooted native dependency decision](15-native-dependency-editing.md) supersedes
planning-only connection semantics in this broader design. Native abandonment in
chains, branches, merges, conflicting children, and another editing workspace is
now tested on jj 0.46.0. Checked [reparenting](18-native-reparenting.md) and
[deletion with retained workspace folders](19-native-node-deletion.md) are implemented.
[Native multi-parent creation](17-rooted-node-dependencies.md#combining-parallel-work)
is also implemented, with clean/conflicted two-/three-parent tests. Editing an
existing merge's parent set and safe filesystem cleanup remain pending.

- Each phase node can be associated with an evolving jj change, initially built on its saved upstream code dependency. Metadata and native ancestry are separate stores that checked edits keep aligned; legacy unbased links remain planning-only.
- Parallel phases are sibling changes on the same parent.
- Simultaneous editing of different nodes needs separate working directories: allocate jj workspaces on demand. Idle nodes and browsing historical revisions do not each require a permanent workspace. Two agents editing different code states must not share one folder.
- Combining parallel results is explicit: for a native merge, create a change with the selected results as parents. jj stores conflicts inside the commit instead of blocking, so a conflicted merge is a state clients can show.
- Editing an earlier phase can cause jj to rebase descendants. Reflect native revisions and conflicts; users infer changed assumptions. Checked downstream restoration bars old affected sessions and reports stale physical checkouts. A separate context-staleness badge or execution gate is outside v1 scope.
- A change ID normally follows evolving work across rewrites; a commit ID identifies one exact revision. Divergence can give a change ID multiple visible revisions and requires an explicit choice.
- jj's operation log supports repository recovery, not undo of workflow metadata, conversations, or external side effects. Workflow edits have their own transactional undo history.

## Browsing and continuing node history

The intended history surface exposes multiple native code revisions without
creating a workflow node for each revision. [Read-only code review](11-backend-cli.md#review-recorded-code-without-snapshotting)
already exposes current node revisions and exact saved run snapshots, including
native changed-file summaries and patches without snapshotting dirty files.
Saved results can now be [restored within the same leaf node/change](21-saved-result-restoration.md).
Waypoints checkpoints current tracked edits, uses native `jj restore` to produce
another revision of the same change, and requires a fresh agent session.
Paginated [native revision history and exact previews](22-native-revision-history.md)
also include manual revisions, divergence and related squash history.
Exact same-change commits, including manual edits/checkpoints, can also be selected
for restoration without a saved run. The visual picker remains unimplemented.
[Confirmed same-workflow downstream restoration](24-downstream-restoration.md)
now checkpoints allocated workspaces, rebases native-aligned descendants,
and bars old affected sessions without rerunning tasks or repairing stale checkouts.
A [readonly restore-impact report](23-restore-impact.md)
lists potential downstream changes/tasks, workspaces and session references
without predicting future conflicts or changing code. `run continue` loads a saved
harness session on the current change; it does not select historical code.

Native code revisions do not preserve historical prompt text or rewind agent memory. Workflow prompt edits are stored separately; matching a prompt version to a code revision requires an explicit association. If an associated native change disappears, retain the saved node with a missing-change indication rather than deleting the workflow or automatically resurrecting repository history.

## Native conflicts versus changed assumptions

jj supplies the code-conflict state. A clean rebase does not prove behavioral
correctness, but Waypoints does not infer a second "context conflict" state.
Users review native changes and decide what to test or rerun; connections never
automatically trigger execution. See the [v1 scope decision](02-architecture.md#decision-reflect-native-jj-changes-not-inferred-context-staleness).

The existing [upstream-revision hint](13-agent-run-lifecycle.md#informational-upstream-hint)
compares direct graph inputs observed before each run with current inputs.
It remains optional bookkeeping and informational output, not a causal tracker,
agent-context verdict, new GUI-indicator requirement, or execution policy.

## Integration approach

The backend shells out to the `jj` CLI. Native jj remains the code-history source
of truth; no duplicate snapshot system is built. Successful agent turns snapshot
automatically after cleanup, with a separate saved/failed outcome. Cancelled or
failed turns preserve files without an automatic snapshot.

## Remaining native/product validation

- Design the visual revision picker and broader downstream reconciliation policy. Native history listing, exact previews, same-change restoration and checked same-workflow downstream application have headless coverage.
- Define editing existing multi-parent dependency sets; creation, conflict materialization, and fake-agent execution/snapshot review now have product-path tests.
- Exercise broader upstream edits through the complete workflow, reflecting native revisions/conflicts and reporting stale physical checkouts without inferring agent-context validity. The narrower clean API/login restore flow passed [real-Codex acceptance](24-downstream-restoration.md#manual-real-codex-acceptance).
- Whether Zed shows a phase's diff in a colocated repo (git HEAD pointing at the working copy's parent).
