# Persistent workflow metadata undo/redo

## Boundary

Waypoints history reverses saved workflow metadata, **not code**. It never runs
jj, restores files, changes native ancestry, reruns an agent, rewinds conversation
memory, grants permission, or rewrites run/snapshot records. Code browsing and
explicit restoration remain separate native jj-backed work.

This implements the first part of the
[workflow history decision](02-architecture.md#architecture-decision-persistent-workflow-undoredo).
Supported actions:

- Rename a workflow.
- Create a prompt node.
- Save a different prompt on an existing node.
- Create a workflow connection in a legacy unbased workflow.
- Move a prompt-only subtree in a rooted based workflow.
- Delete a prompt-only node and reconnect its children to its parent.

The [rooted-node extension](17-rooted-node-dependencies.md) records a based node's
initial base/upstream attachment in the same creation action. Metadata-only
connection edits are blocked in based workflows. [Native reparenting](18-native-reparenting.md)
is a separate acknowledged operation that starts a new undo baseline.

Saving an identical workflow name or prompt text is a no-op: no undo step and no redo-tail deletion.
Rejected edits likewise leave the history untouched.

Workflow creation is setup, not an undoable action in this slice. Associations,
change/workspace allocation, verification, and agent execution are outside this
history and do not move its cursor or discard redo.
[Native node deletion](19-native-node-deletion.md) starts an acknowledged undo
baseline rather than adding an undoable metadata action. Disconnection, ordering,
and layout remain unimplemented.

## Workflow renaming

```sh
waypoints workflow rename --workflow 1 --name "Review authentication"
```

The command returns the updated workflow (`--json` for machine-readable output),
including its unchanged ID, canonical project path, and saved base. Names follow
the same rules as creation: blank/whitespace-only names are rejected, other text
is preserved verbatim, and duplicate names are allowed.

Each changed name adds one `Rename workflow` action to the project's shared
history. Undo/Redo restores only the expected before/after name, not a snapshot
of the whole workflow. Graph attachments, code, workspaces, run records, and
agent sessions are untouched. Renaming and its replay are allowed during active
runs but, like other metadata edits, refuse unresolved native dependency
operations. A changed saved name or missing workflow blocks replay without moving
the cursor. Rename, history append/redo-tail deletion, and cursor writes are atomic.

## Prompt-only move and delete

```sh
waypoints node move-prompt --workflow 1 --node 2 --parent 3
waypoints node move-prompt --workflow 1 --node 2 --from-base
waypoints node delete-prompt --workflow 1 --node 2
```

These explicit metadata-only commands return the updated workflow graph. They
require a rooted based workflow and exactly one existing source parent (node or
base). Legacy unbased/unadopted graphs and multi-parent source nodes are refused.
Move replaces the source's incoming dependency and keeps its subtree. Delete
removes only the source node, reconnects each direct child to the source's parent,
and deduplicates any already-existing inherited attachment. Other child parents
and descendant edges remain intact. A root's children inherit the saved base;
deleting the last node leaves a valid empty rooted workflow.

Both commands refuse any native association, change-creation guard, workspace
allocation, or run record in the source subtree, including completed runs and
incomplete allocations. An upstream destination may already have native code;
the moved subtree may not. No jj queries or mutations are performed, and neither
command can silently switch to the native editing path. Use `node reparent` /
`node delete` explicitly when native code must change.

Each successful edit is one ordinary project-folder metadata history action:
`Move prompt node` or `Delete prompt node`. No undo boundary is cleared.
Moving to the same parent is a no-op and preserves redo. Undo restores the
original identity, exact prompt, and affected attachments; Redo applies the same
saved edit. Deleted identities stay reserved even if a new edit discards redo.
Only affected attachment sets are recorded, not a replacement of the entire graph.

Replay checks the expected saved local state and rechecks subtree references.
If code/workspaces/runs were added afterward, Undo/Redo refuses rather than
discarding them. Cycles, cross-workflow parents, stale history selections,
unresolved native dependency operations, and database failures leave the graph
and history cursor unchanged. Topology writes, node removal/restoration, history
append/redo-tail deletion, and cursor movement share one transaction.

## Decision: acknowledge future code-changing connections

The subsequent [rooted native dependency decision](15-native-dependency-editing.md)
extends this policy to node deletion: abandon native work, reflect jj's descendant
reconnection, and safely clean up its workspace. It replaces the proposal for
metadata-only removal of native-backed nodes. Prompt-only nodes use the undoable
metadata commands above. Native deletion supports deregistering a ready saved
workspace while retaining its folder; filesystem cleanup remains future work.

**Implemented for `node reparent` and `node delete`:** a code-changing dependency operation must
explicitly acknowledge an undo boundary. Reparenting clears the project's old
undo/redo only after preflight passes, atomically with durable operation intent
before native mutation. Uncertain outcomes stay reserved and block further edits;
see [the exact lifecycle and limits](18-native-reparenting.md).
General connect/disconnect and workspace folder cleanup remain future work.
Current legacy `connection create` remains metadata-only and undoable; it does not
rebase code and does not require this acknowledgement.

- **CLI:** require an explicit acknowledgement flag,
  `--accept-undo-boundary`. Explain inline in `--help` that the operation changes
  native code ancestry and that workflow Undo cannot reverse it or cross the
  resulting boundary. Do not require an interactive prompt.
- **Without the flag:** explain the consequence on stderr and exit nonzero
  without changing workflow metadata, native state, or undo history.
- **With the flag:** execute only after normal validation. Acknowledgement is
  not a force option: active writers, unexpected native state, or ambiguous
  parent selection must still block the operation.
- **Shared backend:** enforce the acknowledgement in the RPC/core operation,
  not just CLI argument parsing, so clients cannot bypass it. The eventual GUI
  must communicate the same consequence before requesting the operation.
- **Undo semantics:** the code-changing action establishes a boundary in usable
  workflow history. Ordinary Undo must not silently reverse only the drawn edge
  while leaving the rebase in place, nor run a repository-wide `jj undo`.
  Native code recovery remains a separate explicit operation. This does not
  imply deleting native history.

This is an intentional extension of the original metadata-only connection design,
not permission to silently change the existing command's behavior. Native operation
selection and acknowledgement are distinct concerns; an acknowledgement flag alone
must not turn a metadata-only request into a rebase.

The implemented reparent lifecycle defines exact base selection, history
invalidation, and durable uncertainty when native work succeeds but graph
persistence fails. Clearing history is not a substitute for reconciliation:
unresolved operations remain blocked and inspectable. Explicit reparent recovery
can resolve clear outcomes without changing jj or restoring the old undo/redo
history. General repair and the policy for other native/agent edits invalidating
history remain future work.

## CLI

Select the project folder explicitly:

```sh
waypoints history status --project /path/to/project --json
```

The response contains:

```json
{
  "project_path": "/path/to/project",
  "undo": {"id": 17, "workflow_id": 2, "label": "Edit prompt"},
  "redo": null
}
```

Use the actual advertised edit ID, not these example numbers:

```sh
waypoints history undo --project /path/to/project --expected 17 --json
waypoints history redo --project /path/to/project --expected 17 --json
```

Each returns the new history status. Null means that direction is unavailable.
Without `--json`, the CLI prints the next action names, edit IDs, and affected
workflow IDs. All three commands use the shared backend and can auto-start it.
Rebuild and stop an older backend before using these additive methods.

The required expected ID prevents a client from undoing a different action after
another client edits the project. A stale ID is an error; refresh history status,
inspect the new action, and explicitly choose again. Do not automatically retry.
A lost response is uncertain: inspect both history and workflow state.

## Scope and persistence

There is one chronological history per **canonical project folder**, shared by
all its workflows and all clients. Undo may affect another workflow in that
project; the pending action names its workflow ID. Different projects have
independent histories. This uses existing folder identity, not a new project
registry or automatic grouping of different subfolders of the same repository.

One saved edit is one undo step. Normal text-input undo while typing belongs to
the eventual GUI. Undo applies recorded before-values; redo restores recorded
after-values and node identities. It never reruns the original public command.
A new supported edit after undo atomically drops that project's redo tail.
Selective undo and branching undo histories are not supported.

Metadata edits, history insertion/tail deletion, and cursor updates share one
database transaction. Undo and redo also mutate metadata and move the cursor in
one transaction. Failed writes roll back both. Backend requests serialize these
operations; no transaction includes a subprocess or agent wait.

Tables:

- `workflow_history_heads`: project path and current cursor.
- `workflow_edits`: stable edit ID, project-local position, format version, and
  a serialized typed action with before/after values.
- `node_identities`: permanently reserved IDs, including IDs temporarily absent
  while creation is undone. A different project cannot reuse a redo identity.

Current graphs load directly from the existing tables, not by replaying history.
Old databases receive additive tables. Their existing contents form a baseline:
no fabricated prior edits, and only subsequent supported edits are undoable.
Old prompt text is intentionally retained in history records.

Records have explicit format versions 1 (original metadata actions), 2
(attachment-aware node creation), 3 (prompt-only move/delete), 4 (workflow
renaming), and 5 (multi-parent node creation). Earlier versions remain readable;
older executables cannot replay newer record versions. Multi-parent creation
records all incoming attachments as one `Create node` action, with the same
native-reference protections as single-parent creation.
Unsupported versions or malformed actions
produce an error when encountered, without moving the cursor or deleting current
workflow state. This slice does not implement format conversion or a history-reset
command; future upgrades must migrate or explicitly invalidate incompatible history.

## Node references and active runs

Undoing node creation removes only that node's metadata row. It is refused if
the node's saved values changed or if any of these references depend on it:

- Workflow connections that have not yet been undone.
- A jj change association.
- A pending native change-creation guard.
- A saved workspace allocation.
- Any run record, including completed or interrupted runs.

The cursor stays put; no references, files, native changes, or run history are
removed to make undo work. History is chronological, so a blocked creation cannot
be skipped to undo earlier actions. [Native deletion](19-native-node-deletion.md)
is a separate acknowledged operation, not an implicit cleanup policy to make
metadata undo succeed. Saved workspace deregistration is part of native deletion,
not an implicit action performed to make metadata Undo succeed.

Prompt undo/redo is allowed while a run is active. A run has already captured its
assignment: changing the saved node prompt does not change the running prompt,
session, approvals, or eventual snapshot. Only legacy unbased connections are
metadata-undoable; undoing one does not cancel runs or change jj parents.

## RPC

| Method | Parameters | Result |
|---|---|---|
| `workflow.rename` | `{workflow_id, name}` | Updated saved workflow; one undoable name change |
| `history.status` | `{project_path}` | `{project_path, undo, redo}` |
| `history.undo` | `{project_path, expected_edit_id}` | New history status |
| `history.redo` | `{project_path, expected_edit_id}` | New history status |

Send absolute project paths at the RPC boundary; clients resolve relative paths
from their own working directory. Extra fields and missing/incorrectly typed parameters are rejected.
Empty history, stale selections, blocked actions, unexpected saved values, and
unsupported history formats are domain errors. Existing error/transport limits
and the no-automatic-retry rule apply.

## Validation

Automated tests cover:

- Undo/redo of all three action types with exact prompt values and node identities.
- Cursor and graph persistence across database/backend restart.
- Chronology across workflows, project isolation, and reserved redo identities.
- Redo-tail replacement; rejected/no-op edits preserving redo.
- Stale action IDs, including a replaced action at the same cursor position.
- SQL failures after metadata mutation/tail deletion, and cursor-write failures
  after undo/redo mutation, rolling back state and history together.
- Unsupported history versions and mismatched current metadata refusing undo.
- Additive upgrade of old nodes as a baseline.
- Real CLI and raw RPC sharing the same persisted history.
- Native jj operation/commit IDs, dirty files, checkout metadata, agent requests,
  active run state, and permission state unchanged by prompt undo/redo.
- Refusal to undo creation with native/workspace/run references, even after a run stops.

These checks use disposable databases and fake agents; no real Codex session or
GUI is required to validate metadata history.
