# Rooted node creation and native starting dependencies

## Implemented scope

New nodes in a **based workflow** must explicitly build on its saved base or an
existing upstream node. Node creation and its attachment are one database
transaction and one metadata undo step. No orphan is saved between them.

The base is not a numbered prompt node. The graph exposes `base_nodes` for nodes
attached directly to it, plus existing node-to-node `connections`.

Saving a prompt node does not immediately create code or a workspace. When
requested, native change creation uses its saved dependency.
[Prompt-only move/delete](14-workflow-history.md#prompt-only-move-and-delete)
can rearrange an unmaterialized subtree with ordinary metadata Undo/Redo.
[Native reparenting](18-native-reparenting.md) now moves existing native nodes
with an acknowledged undo boundary. Native merge creation, disconnecting,
and workspace folder cleanup remain future operations.
[Native deletion](19-native-node-deletion.md) now abandons a node and reconnects
its children, deregistering its ready saved workspace without removing files.
The existing metadata-only
connection command is refused in based workflows; it cannot bypass these rules.

## CLI

Create a based workflow, then nodes attached to the base or an upstream node:

```sh
waypoints workflow create --project . --name "Implementation" --base main --json
# Use the actual workflow and node IDs returned by these commands.
waypoints node create --workflow 1 --prompt "Interfaces" --from-base --json
waypoints node create --workflow 1 --prompt "Authentication" --parent 1 --json
waypoints node create --workflow 1 --prompt "Images" --parent 1 --json
waypoints workflow graph --workflow 1 --json
```

Exactly one parent choice is required in a based workflow; the flags are mutually
exclusive. A parent must already exist in the same rooted workflow. Parents are
not inferred from prompt text, creation order, the current checkout, or bookmarks.
Because the child is new, attaching it to an existing rooted node cannot introduce
a cycle. Multiple children of one parent are allowed.

Graph fields include:

```json
{
  "base_nodes": [1],
  "connections": [
    {"workflow_id": 1, "source_id": 1, "target_id": 2},
    {"workflow_id": 1, "source_id": 1, "target_id": 3}
  ],
  "connectivity": {"status": "rooted"}
}
```

`connectivity` describes **saved reachability**, not a guarantee that external jj
commands have left native ancestry aligned or physical checkouts fresh.

When ready, create native work without making a second independent base choice:

```sh
waypoints node create-change --workflow 1 --node 1 --json
waypoints node create-change --workflow 1 --node 2 --json
waypoints node create-change --workflow 1 --node 3 --json
```

- A base-attached node starts on the exact saved workflow commit, even if `main`
  has advanced.
- An upstream-node dependency resolves that node's current unique native revision
  when creating the child's code. The upstream change must exist and be idle;
  missing/divergent/unassociated or reserved upstream work blocks creation.
- Explicit `create-change --base ...` remains accepted, but in a based workflow
  it must resolve to the same exact dependency revision. A mismatch fails before
  the native creation guard or command.
- Native creation currently requires exactly one parent. A legacy fan-in graph
  may be adopted as prompt metadata, but native multi-parent creation is not yet
  supported.
- Existing durable creation guards and uncertain-outcome recovery still apply.
  Code creation does not switch or snapshot the original working copy.

Explicit `node associate` in a based workflow checks the selected change's actual
native parent commits against its saved dependencies. It never rebases a mismatched
selection to make it fit. A change owned by a based node cannot also be assigned
to another node, including through a legacy workflow.

These are creation/association checks, not continuous monitoring of external jj
edits. General ancestry reconciliation and downstream staleness are still pending.

## Undo/redo

Creating a prompt with its initial attachment is metadata-only: no rebase and no
undo-boundary acknowledgement are needed.

Undo removes the node and its own incoming attachment together; redo restores the
same identity and dependency. A failed history/cursor write rolls back all rows.
Outgoing child connections, changed dependencies, native associations, workspace
allocations, creation guards, and run records block removal rather than being erased.
Failed undo restores any attachment temporarily removed inside its transaction.

New attachment-aware creation records use history format version 2; existing
version 1 records remain readable. An old creation record without an attachment
cannot be redone as an orphan into a now-based workflow. Old metadata-only
connection undo/redo is blocked once the workflow has a base, so it cannot orphan
adopted nodes or masquerade as a native dependency edit. Prompt edits remain
undoable and never rewind code or agent memory.

## Existing workflows

No saved graph is silently adopted or rebased.

- Workflows without a base remain in legacy compatibility mode:
  `connectivity.status = "unbased"`. Old prompt/connection operations still work.
  To use rooted dependencies, select a base explicitly.
- Selecting a base for a nonempty old graph produces
  `{"status":"needs_adoption","node_ids":[...]}` until every node has an explicit
  path to it. New node/dependency creation is blocked in that state.
- An empty based workflow is already rooted.

For an existing **prompt-only** graph, explicitly attach all its graph roots:

```sh
waypoints workflow set-base --workflow 1 --base main --json
waypoints workflow graph --workflow 1 --json
waypoints workflow adopt-roots --workflow 1 --base-nodes 1,4 --json
```

Choose the actual existing nodes with no incoming connections. The selection must
include all and only those roots, without duplicates. Adoption retains existing
nodes, prompts, and edges; it only adds the explicit base attachments, atomically.
Incomplete selections fail unchanged. It does not clear earlier metadata history,
but the dependency protections above may make old creation/connection actions
unavailable. Adoption itself is not a new undo action.

Graphs with any native association, workspace, creation guard, or run history are
**refused** by this prompt-only adoption command. Their safe native reconciliation
is a future slice. They remain inspectable and their code/files/records are retained;
do not delete associations or edit database tables to bypass this refusal.

## API and storage

| RPC method | Parameters | Result |
|---|---|---|
| `node.create` | `{workflow_id, prompt, parent?}` | Saved node |
| `workflow.adopt_roots` | `{workflow_id, base_nodes}` | Adopted workflow graph |
| `node.create_change` | `{workflow_id, node_id, base?}` | Native node change |

Parent shapes:

- `{"kind":"workflow_base"}`
- `{"kind":"node","node_id":1}`

Missing parents are allowed only for legacy unbased workflows. Unknown fields,
wrong shapes, missing/foreign parent nodes, and unadopted graphs are rejected by
the backend/core, not just CLI argument parsing.

`workflow_base_nodes` is an additive table holding explicit base-to-node
attachments. Node-to-node attachments reuse `connections`; there is no parallel
dependency-edge store. Reachability is computed from saved roots and edges.
Client/backend restart does not reconstruct or infer any new connection.

The API remains protocol version 1 with additive methods/fields. Rebuild and
stop an older backend before using these commands.

## Validation

Disposable native/core and real CLI/RPC tests cover explicit roots and parents,
fan-out, missing/foreign parents, orphan refusals, restart, attachment-aware
undo/redo, SQL-failure rollback, malformed selectors, legacy adoption refusals,
native dependency-based creation, wrong-base/parent refusals, and exclusive native
change ownership. Metadata operations leave native operation IDs unchanged.
