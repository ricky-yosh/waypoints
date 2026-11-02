# Rooted node creation and native starting dependencies

## Implemented scope

New nodes in a **based workflow** must explicitly build on its saved base or one
or more existing upstream nodes. Node creation and all its attachments are one database
transaction and one metadata undo step. No orphan is saved between them.

The base is not a numbered prompt node. The graph exposes `base_nodes` for nodes
attached directly to it, plus existing node-to-node `connections`.

Saving a prompt node does not immediately create code or a workspace. When
requested, native change creation uses its saved dependency.
[Prompt-only move/delete](14-workflow-history.md#prompt-only-move-and-delete)
can rearrange an unmaterialized subtree with ordinary metadata Undo/Redo.
[Native reparenting](18-native-reparenting.md) now moves existing native nodes
with an acknowledged undo boundary. Editing an existing merge's parent set, disconnecting,
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

Choose `--from-base` alone or one or more `--parent` values in a based workflow.
Repeat `--parent` or use a comma-separated list. Each parent must be distinct and
already exist in the same rooted workflow; mixing the base with nodes is refused. Parents are
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
- Explicit `create-change --base ...` remains accepted for a single dependency,
  but it must resolve to the same exact dependency revision. A mismatch fails before
  the native creation guard or command.
- Multiple saved upstream nodes produce an ordinary native merge change using
  their exact recorded commit IDs. The created change's native parent set is
  verified before saving its association. Duplicate native revisions are refused.
- Existing durable creation guards and uncertain-outcome recovery still apply.
  Code creation does not switch or snapshot the original working copy.

Explicit `node associate` in a based workflow checks the selected change's actual
native parent commits against its saved dependencies. It never rebases a mismatched
selection to make it fit. A change owned by a based node cannot also be assigned
to another node, including through a legacy workflow.

These are creation/association checks, not continuous monitoring of external jj
edits. General ancestry reconciliation remains unimplemented.
[V1 reflects native changes/conflicts](02-architecture.md#decision-reflect-native-jj-changes-not-inferred-context-staleness),
not inferred downstream-context validity.

## Combining parallel work

After nodes 2 and 3 each have a native change:

```sh
waypoints node create --workflow 1 --prompt "Integrate and test" --parent 2 --parent 3 --json
# Use the returned node ID, shown as 4 here.
waypoints node create-change --workflow 1 --node 4 --json
waypoints node workspace create --workflow 1 --node 4 --directory /new/absolute/integration-workspace --json
```

Three or more parents work the same way, for example `--parent 1,2,3`.
This is a normal prompt node, not a special merge-node type. Saving the prompt
only records dependencies; `create-change` performs native `jj new --no-edit`
with all resolved parents, without snapshotting or switching the original checkout.
Each input must have one current native revision and no active/interrupted writer.
Unsnapshotted upstream files are not merged. A single explicit `--base` cannot
override a multi-parent selection.

jj combines the recorded trees. Clean merges contain all nonconflicting work.
Conflicted merges are saved successfully with `revision.conflicted: true`;
allocating their workspace materializes native conflict markers. The user or
agent must explicitly resolve and test that code. Creation is not approval and
does not automatically repair conflicts. Existing workspace/run/snapshot and
read-only node/run review paths also work for these nodes; diffs compare against
the merged parent tree, not a separate diff for every input branch.

Incoming attachments are a set, stored in stable graph order, not a branch
priority list. Existing reparent/delete commands still require a single-parent
source node; this slice does not edit or remove a merge's parent set.

## Undo/redo

Creating a prompt with its initial attachment is metadata-only: no rebase and no
undo-boundary acknowledgement are needed.

Undo removes the node and all its own incoming attachments together; redo restores the
same identity and dependency set. A failed history/cursor write rolls back all rows.
Outgoing child connections, changed dependencies, native associations, workspace
allocations, creation guards, and run records block removal rather than being erased.
Failed undo restores any attachment temporarily removed inside its transaction.

Single-parent creation records keep history format version 2; multi-parent
creation uses version 5. Existing versions 1–4 remain readable. An old creation record without an attachment
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
| `node.create` | `{workflow_id, prompt, parent?, parents?}` | Saved node; select one parent shape or a nonempty array, not both |
| `workflow.adopt_roots` | `{workflow_id, base_nodes}` | Adopted workflow graph |
| `node.create_change` | `{workflow_id, node_id, base?}` | Native node change |

Parent shapes:

- `{"kind":"workflow_base"}`
- `{"kind":"node","node_id":1}`

The existing `parent` field remains compatible. `parents` is a non-null, nonempty
array of the same shapes. `Storage::create_node_from_parents` and
`Client::create_node_from_parents` expose the array path; the single-parent APIs remain.
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
change ownership. Two-/three-parent tests cover clean and conflicted merges,
exact parent sets, unchanged dirty original checkouts, workspace materialization,
busy-input refusals, fake-ACP execution, upstream inputs, snapshot review after
restart, and history rollback/redo without jj. Metadata operations leave native
operation IDs unchanged.

### Manual Codex acceptance

A disposable two-parent **clean merge** also passed with `codex-acp` 2.5.0 and
jj 0.46.0 using the installed agent's existing authentication:

- A fixed base contained four Python integration tests and a task contract.
- Independent native branches added a greeting API and an HTML-escaping renderer.
- A normal node selected both parents. Its dedicated workspace contained both
  recorded implementations, and the integration tests initially failed because
  the integration module did not exist.
- Codex received an explicit prompt explaining the two inputs, added only
  `app.py`, and ran `python3 -B -m unittest discover -s tests -v`: all four passed.
  The same exact command passed independently afterward. Both input files and
  the original tests/contract were unchanged.
- Run 1 completed with snapshot
  `0071157abd40cb85f96fc80c81ada8306743037d`, retaining both parent commits
  `5d66877da1cb24a92707175134a11989cadd2b03` and
  `830845c912ced7b46de8aa3dbe49ec486e153157`.
- After stopping/restarting the backend, run review returned that exact snapshot
  and both parents. Node review matched it; the diff added only `app.py`. The
  original checkout and native operation head were unchanged by review.

The disposable database/native repository and JSON evidence were left under the
ignored `target/merge-acceptance/` directory; the agent workspace was turn-local
scratch space. The test backend was stopped. No credentials were copied.
The bounded event buffer reported a gap, so its saved tail is not a complete
transcript; it did retain the test command, four passing results, and exit code 0.

Environment notes: an attempted workspace under the Waypoints checkout was
correctly refused because it lay inside an existing jj workspace. Initial setup
also encountered a temp-directory I/O error across Delta terminal invocations;
the successful disposable backend used stable `TMPDIR=/tmp` rather than the
per-command temporary directory. No production workaround was added; inherited
temporary-directory lifetime is a separate reliability follow-up.

This does not manually validate three-parent integration, agent-driven conflict
resolution, automatic upstream-context injection, or the full upstream-edit/
historical-code-continuation acceptance scenario.
