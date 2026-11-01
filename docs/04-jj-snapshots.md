# jj snapshots per phase

See [core/backend status](20-core-backend-status.md) for the capability inventory.
This page separates implemented native integration from broader history/staleness design.

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
The model below remains the broader design, not a claim that history navigation,
downstream staleness, or workflow-level checkpoints are implemented.

## Model

The [rooted native dependency decision](15-native-dependency-editing.md) supersedes
planning-only connection semantics in this broader design. Native abandonment in
chains, branches, merges, conflicting children, and another editing workspace is
now tested on jj 0.46.0. Checked [reparenting](18-native-reparenting.md) and
[deletion with retained workspace folders](19-native-node-deletion.md) are implemented.
Native multi-parent creation and safe filesystem cleanup remain
pending; native primitive tests do not establish those product capabilities.

- Each phase node can be associated with an evolving jj change, initially built on its saved upstream code dependency. Metadata and native ancestry are separate stores that checked edits keep aligned; legacy unbased links remain planning-only.
- Parallel phases are sibling changes on the same parent.
- Simultaneous editing of different nodes needs separate working directories: allocate jj workspaces on demand. Idle nodes and browsing historical revisions do not each require a permanent workspace. Two agents editing different code states must not share one folder.
- Combining parallel results is explicit: for a native merge, create a change with the selected results as parents. jj stores conflicts inside the commit instead of blocking, so a conflicted merge is a state clients can show.
- Editing an earlier phase can cause jj to rebase descendants. Native conflict flags are queryable now; marking downstream agent context stale and displaying badges remain product work.
- A change ID normally follows evolving work across rewrites; a commit ID identifies one exact revision. Divergence can give a change ID multiple visible revisions and requires an explicit choice.
- jj's operation log supports repository recovery, not undo of workflow metadata, conversations, or external side effects. Workflow edits have their own transactional undo history.

## Browsing and continuing node history

The intended history surface exposes multiple native code revisions without
creating a workflow node for each revision. [Read-only code review](11-backend-cli.md#review-recorded-code-without-snapshotting)
already exposes current node revisions and exact saved run snapshots, including
native changed-file summaries and patches without snapshotting dirty files.
General native revision browsing and explicit historical-code continuation are
not implemented. Continuing work from an old revision requires an
explicit native operation whose restore/branch behavior and retention still need
agreement and validation. Implemented `run continue` loads a saved harness session
on the current change; it is not historical-code continuation.

Native code revisions do not preserve historical prompt text or rewind agent memory. Workflow prompt edits are stored separately; matching a prompt version to a code revision requires an explicit association. If an associated native change disappears, retain the saved node with a missing-change indication rather than deleting the workflow or automatically resurrecting repository history.

## Two kinds of conflict

1. **Code conflicts:** jj handles these.
2. **Context staleness (not fully tracked):** downstream reasoning or work was built on old upstream output even if the code merges cleanly. jj cannot see reasoning. A small [upstream-revision hint](13-agent-run-lifecycle.md#informational-upstream-hint) compares direct graph inputs observed before each run with current inputs. This is informational only, not causal/transitive context tracking or a rerun policy. Connections do not automatically trigger execution.

## Integration approach

The backend shells out to the `jj` CLI. Native jj remains the code-history source
of truth; no duplicate snapshot system is built. Successful agent turns snapshot
automatically after cleanup, with a separate saved/failed outcome. Cancelled or
failed turns preserve files without an automatic snapshot.

## Remaining native/product validation

- Define native revision-history browsing beyond saved runs. Current read-only review has automated coverage for bounded output, rewritten/abandoned snapshots, missing native objects, and untouched dirty files.
- Validate multi-parent product creation/editing beyond the already tested native abandonment primitives.
- Exercise upstream edits through the complete workflow and define context-staleness behavior separately from native conflicts and stale physical checkouts.
- Whether Zed shows a phase's diff in a colocated repo (git HEAD pointing at the working copy's parent).
