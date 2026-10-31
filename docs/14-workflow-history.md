# Persistent workflow metadata undo/redo

## Boundary

Waypoints history reverses saved workflow metadata, **not code**. It never runs
jj, restores files, changes native ancestry, reruns an agent, rewinds conversation
memory, grants permission, or rewrites run/snapshot records. Code browsing and
explicit restoration remain separate native jj-backed work.

This implements the first part of the
[workflow history decision](02-architecture.md#architecture-decision-persistent-workflow-undoredo).
Supported actions:

- Create a prompt node.
- Save a different prompt on an existing node.
- Create a workflow connection.

Saving identical prompt text is a no-op: no undo step and no redo-tail deletion.
Rejected edits likewise leave the history untouched.

Workflow creation is setup, not an undoable action in this slice. Associations,
change/workspace allocation, verification, and agent execution are outside this
history and do not move its cursor or discard redo. Node deletion/disconnection,
ordering, and layout are not implemented yet.

## Decision: acknowledge future code-changing connections

**Agreed direction, not implemented:** a future connect/disconnect operation
that rebases native jj work must explicitly acknowledge an undo boundary.
Current `connection create` remains metadata-only and undoable; it does not
rebase code and does not require this acknowledgement.

- **CLI:** require an explicit acknowledgement flag, planned as
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

Before implementing this extension, define the exact rebase/base-selection rules,
how affected histories are invalidated, and crash reconciliation when native work
succeeds but metadata persistence fails. Clearing history after failed or uncertain
operations must not be used as a substitute for reconciliation. The policy for
other native/agent edits invalidating workflow history remains to be specified.

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

Records have explicit format version 1. Unsupported versions or malformed actions
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
be skipped to undo earlier actions. General deletion with retained native/run
references is a separate future design, not an implicit native cleanup policy.

Prompt undo/redo is allowed while a run is active. A run has already captured its
assignment: changing the saved node prompt does not change the running prompt,
session, approvals, or eventual snapshot. Connections are workflow intent only;
undoing one does not cancel runs or change jj parents.

## RPC

| Method | Parameters | Result |
|---|---|---|
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
