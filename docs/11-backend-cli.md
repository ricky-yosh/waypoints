# Local backend and CLI

## Quick start

No database server or manual backend startup is required:

```sh
cargo run -p waypoints-cli -- workflow create --project . --name "First workflow"
cargo run -p waypoints-cli -- workflow list --project . --json
cargo run -p waypoints-cli -- backend status --json
cargo run -p waypoints-cli -- backend stop
```

`--json` prints a single JSON value on stdout. Errors go to stderr and exit
nonzero; command syntax errors use Clap's exit status 2. A workflow name may be
duplicated, but it must not be blank. Projects must be existing directories; both
clients and storage resolve their paths so relative paths don't depend on the
background server's working directory.

## Save a branching workflow

Nodes contain saved prompts, with optional explicit associations to existing jj changes.
They do not store agent conversations.
Connections are directed saved relationships: they do not execute work or change
jj ancestry. All commands below use the backend, never the database directly.

First create or list workflows to find the workflow ID. For a new empty database
the first workflow has ID 1; use the actual IDs returned by your commands.

```sh
cargo run -p waypoints-cli -- node create --workflow 1 --prompt "Interfaces + placeholders"
cargo run -p waypoints-cli -- node create --workflow 1 --prompt "Authentication"
cargo run -p waypoints-cli -- node create --workflow 1 --prompt "Image uploading"
cargo run -p waypoints-cli -- node create --workflow 1 --prompt "Settings"
```

Using the returned node IDs (1–4 in a new database):

```sh
cargo run -p waypoints-cli -- connection create --workflow 1 --source 1 --target 2
cargo run -p waypoints-cli -- connection create --workflow 1 --source 1 --target 3
cargo run -p waypoints-cli -- connection create --workflow 1 --source 1 --target 4
cargo run -p waypoints-cli -- node update --workflow 1 --node 2 --prompt "Implement authentication and tests"
cargo run -p waypoints-cli -- node get --workflow 1 --node 2 --json
cargo run -p waypoints-cli -- workflow graph --workflow 1 --json
```

Graph JSON contains `workflow`, `nodes`, and `connections`. Nodes are ordered by
ID; connections by source then target ID. Node IDs are database-local and stable
when prompts are edited. Prompts are preserved verbatim, but blank/whitespace-only
prompts are rejected. Missing workflows/nodes, cross-workflow links, self-links,
duplicate links, and directed cycles return errors without changing the graph.
Fan-out and fan-in are allowed. Nodes need not be connected.

The database adds `nodes` and `connections` tables when opening an existing
workflows-only database, preserving saved workflows. Graphs and prompt edits
survive backend restarts. Prompt updates currently replace the saved text:
there is **no undo or prompt revision history yet**, and no delete/disconnect
command in this slice.

After rebuilding, stop any old backend and let the next workflow/node/connection
command start the new executable. Protocol version 1 is retained for these
additive methods; an older running backend reports "method not found" for them.

## Associate a node with an existing jj change

These commands require `jj` on the **backend's PATH**. Tested with jj 0.46.0;
older versions need the `json()` template function, `change_id()` revset function,
and the global flags used below. Basic workflows and graphs do not require jj.
The workflow's project directory must be inside an existing jj workspace.

First inspect change IDs in that project, without snapshotting dirty files:

```sh
jj --ignore-working-copy --at-op=@ log
```

Select one existing change by its change ID (letters k–z), or a unique prefix.
Replace `CHANGE_ID` and the workflow/node IDs with your actual values:

```sh
cargo run -p waypoints-cli -- node associate --workflow 1 --node 2 --change CHANGE_ID
cargo run -p waypoints-cli -- node refresh --workflow 1 --node 2 --json
```

Commit IDs, bookmarks, `@`, and arbitrary revsets are intentionally not accepted.
An ambiguous prefix, nonexistent change, or divergent change cannot be selected.
Selection resolves the full change ID and stores it with the canonical jj workspace
root in the `node_changes` table. Running `associate` again explicitly replaces that
node's association; a rejected selection leaves its prior association unchanged.
Neither selection nor refresh changes workflow connections or jj ancestry.

`node refresh` returns `{node, association, state}`. `association` is either null
or `{repository_path, change_id}`. `state.status` is:

| Status | Meaning |
|---|---|
| `unassociated` | No native change selected; no jj query is needed |
| `current` | One visible revision; `state.revision` contains full change/commit IDs, description, and `conflicted` |
| `missing` | No visible revision; the saved node, prompt, links, and association are retained |
| `divergent` | Multiple visible revisions; `state.revisions` contains two candidates, not an exhaustive history |

Revision details are queried live, not cached or preserved as application-owned
snapshots. `node get` and `workflow graph` still return only saved prompt/graph
metadata and do not query jj; use `node refresh` for native status.

After a manual native rewrite (for example `jj describe`), refresh resolves the
same change ID to the new commit/description/conflict state. After external
abandonment or squash removes a change, refresh reports `missing`; nothing is
deleted or resurrected. Repository access failures, a changed workspace root,
unsupported jj versions, missing jj, and concurrent operation heads are **errors**,
not evidence that a change disappeared. Resolve native operation-head concurrency
in jj itself, then retry. An explicitly replaced repository at the same path is
not identified independently; repository relocation/replacement tracking is deferred.

Queries use `--ignore-working-copy --at-op=@` to avoid automatic working-copy
snapshots, Git import, and concurrent-operation merging. They therefore reflect
jj's recorded state, **not unsnapshotted file edits**. Each subprocess has a
two-second timeout and a 1 MiB limit per output stream; slow/oversized queries
return an error. The owning process is terminated on timeout.

Associations survive backend restarts. Neither association nor refresh creates
changes. Explicit change and workspace creation are described below.
Switching existing working copies, revision-history browsing, code restoration,
downstream staleness, and agent execution are not implemented.

## Create a change for an unassociated node

Choose a base **explicitly**; workflow connections do not infer jj ancestry.
`BASE_REVISION` accepts one existing change ID, commit ID, unique ID prefix, or
`root()`. Bookmarks, `@`, and arbitrary revsets are not accepted. A missing or
divergent change cannot be a base; select an exact commit ID to disambiguate.

```sh
cargo run -p waypoints-cli -- node create-change --workflow 1 --node 2 --base BASE_REVISION --json
cargo run -p waypoints-cli -- node refresh --workflow 1 --node 2 --json
```

The command resolves the base to an exact commit, runs native `jj new --no-edit`
with an empty description and `--ignore-working-copy --at-op=@`, and saves the
new full change ID and workspace root. Its response has the same
`{node, association, state}` shape as refresh. The node's prompt and connections
are unchanged. The new change initially has the base's tree, with no added edits.
Your current working-copy change, recorded commit, and dirty files stay unchanged.
Two nodes created on the same base are native siblings even if a workflow edge
connects them.

Creation is refused for **any existing association**, including a missing change.
It does not replace, resurrect, or switch to an associated change. These
associations also survive backend restarts.

jj and the workflow database cannot commit atomically. Before invoking native
creation, Waypoints saves a durable guard in `node_change_creations`. A failed or
interrupted operation after that point leaves the guard in place, so retrying
cannot silently create another change. A four-second total native-work deadline
fits within the five-second client deadline; each subprocess retains its
two-second timeout and output limits.

If an outcome is uncertain, **inspect jj before doing anything else**:

```sh
jj --ignore-working-copy --at-op=@ log
jj --ignore-working-copy --at-op=@ op log
```

If the new change exists, use `node associate --workflow … --node … --change …`
to explicitly attach it. Successful association clears the guard. If creation
never happened, you can manually create the intended change in jj and associate
it. There is no automatic rollback or force-retry command. Failure to save an
association after successful native creation reports the created change ID.

Creation is tested with jj 0.46.0. It overrides jj's commit-summary template with
marked JSON to capture the identity from **that creation command**, rather than
guessing from the newest repository head. Unsupported output produces an error
and retains the guard instead of guessing.

## Allocate a node's editing workspace

Workspace allocation is **explicit and on demand**, not automatic for every node.
First associate or create the node's change. Then choose a new directory with an
existing parent, outside other jj workspaces:

```sh
cargo run -p waypoints-cli -- node workspace create --workflow 1 --node 2 --directory ../my-app-auth --json
cargo run -p waypoints-cli -- node workspace status --workflow 1 --node 2 --json
```

Relative destinations are resolved from the CLI's current directory, never the
background backend's working directory. The core and RPC require an absolute
path; the existing parent is canonicalized before saving. Destinations that
already exist (even empty directories, files, or dangling symlinks), missing
parents, and nesting within an existing jj workspace are refused. Waypoints
atomically claims the new directory after saving the allocation; it never clears
or adopts an occupied destination.

The node must have one visible, mutable native revision. Missing, divergent,
unassociated, or immutable changes cannot be opened this way. A registered
workspace already editing the same change is also refused, including the original
working copy. Use that existing workspace directly, or select a different change.
These checks do not lock out independent native jj users; the repository history
is still shared.

The returned shape is `{node, workspace, state}`. `workspace` is null or contains
`{repository_path, change_id, name, directory}`. The generated native name and
explicit directory are saved in `node_workspaces`, and survive backend restarts.

| `state.status` | Meaning |
|---|---|
| `unallocated` | No workspace has been requested; no native query is needed |
| `ready` | Registry and recorded working-copy revision match the node; `state.revision` contains the native revision |
| `incomplete` | Creation was reserved but not confirmed complete; directory/name remain available for inspection |
| `missing_directory` | The saved directory disappeared; nothing is recreated |
| `mismatched` | A workspace was forgotten, switched, or replaced, or the node's association changed; `state.detail` explains |

`ready` describes **recorded jj state**, not a claim that dirty files were
snapshotted or that a checkout affected by external rewrites is fresh. Status
does not run `jj status` or `workspace update-stale`. Native access/permission
errors and unavailable jj remain errors, not missing-directory indications.
Repository replacement at the same path is not independently tracked.

Open the returned directory in your editor or existing harness. Native jj use
there records edits into the node's existing change:

```sh
cd ../my-app-auth
# Edit files here, then explicitly ask jj to record them.
jj status
# Run Waypoints with your normal database/socket configuration:
waypoints node refresh --workflow 1 --node 2 --json
```

Waypoints does not launch an editor or agent. Two node workspaces on sibling
changes have separate files, but native ancestry and operation history are shared;
rewriting an ancestor can still rebase descendants or introduce conflicts.

### Repeat requests and partial creation

Repeating `workspace create` for the same node and directory **only reports the
saved allocation and current status**. It does not snapshot, update, repair, or
retry native work, even if the directory is missing or creation is incomplete.
A different directory is refused. Reassociating the node does not retarget or
discard its saved workspace.

Creation has three explicit native steps: add a non-colocated, full-tree
workspace; edit the selected exact node revision; and materialize the new
directory with `workspace update-stale`. `workspace add -r` alone would edit a
new child, which is not the desired behavior. Add/edit ignore the original
working copy; only the final command updates files, in the **new** directory.
The temporary empty child is discarded by native jj during edit.

The allocation is saved as incomplete **before** any directory/native creation.
jj and Turso cannot commit atomically. Failed checkout, timeout, process crash,
or failed final database write therefore retains the path, native name, and any
partial files. Inspect them rather than deleting or blindly recreating:

```sh
waypoints node workspace status --workflow 1 --node 2 --json
jj --ignore-working-copy --at-op=@ workspace list
jj --ignore-working-copy --at-op=@ op log
```

Native recovery is manual in this slice. No Waypoints finalize, forget, replace,
or deletion command is implemented; an incomplete allocation remains marked
incomplete even after manual native repairs. This intentionally favors retaining
work over automatic cleanup or uncertain retries.

Workspace creation has a 110-second total allocation deadline and a 60-second
limit per mutation subprocess, with the same 1 MiB per-stream output bound.
The client allows 120 seconds for this method; ordinary requests keep their
five-second deadline. Workspace status has a four-second native-query budget.
The backend still serializes requests, so a long checkout can delay other
requests. Timing out a client never authorizes automatic retry.

Tested with jj 0.46.0, including workspace root templates, `--no-colocate`,
`--sparse-patterns full`, and `workspace update-stale`.

## Data and socket paths

Defaults:

- If set, `$XDG_STATE_HOME/waypoints/workflows.db`.
- Otherwise on macOS, `$HOME/Library/Application Support/waypoints/workflows.db`.
- Otherwise on Linux, `$HOME/.local/state/waypoints/workflows.db`.

The default socket is alongside the database, named `workflows.sock`. Newly
created parent directories have mode 0700; the socket and newly created lock/log
files have mode 0600. Existing directories are not chmodded.

For isolated development data, consistently pass the same overrides to every
command:

```sh
cargo run -p waypoints-cli -- --database ./dev-data/workflows.db --socket ./dev-data/backend.sock workflow create --project . --name "Test"
cargo run -p waypoints-cli -- --database ./dev-data/workflows.db --socket ./dev-data/backend.sock workflow list --project . --json
cargo run -p waypoints-cli -- --database ./dev-data/workflows.db --socket ./dev-data/backend.sock backend stop
```

Unix socket paths have a short OS-dependent length limit (about 100 bytes on
macOS). If necessary, choose a shorter socket path in a directory you own.
Local development data is not source code: do not commit database, socket, lock,
or log files.

## Lifecycle

Workflow commands first connect and verify the database path and protocol version.
Only a missing or refused socket triggers a background `waypoints … serve`
process. A live but unresponsive or mismatched backend produces an error instead
of being replaced. Readiness waits are bounded; startup errors are written to
`<socket>.log`. The server runs independently of the CLI's terminal process group
and remains available until stopped or interrupted.

The server holds advisory locks at `<database>.lock` and `<socket>.lock` before
opening storage or touching an existing socket. Concurrent starters may briefly
launch competing processes, but only one opens the database; the others exit and
their clients connect to the winner. Lock files are deliberately never unlinked.
Locks release when the process exits, including after a crash.

A stale socket is removed only after acquiring both ownership locks and checking
that it refuses connections. A regular file, symlink, or live listener at that path
is not removed. `backend status` and `backend stop` do not start a backend.
Shutdown normally removes the socket; the database and lock/log files remain.

For foreground debugging only:

```sh
cargo run -p waypoints-cli -- serve
```

Ctrl+C stops a foreground server. This isn't a system service, and there is no
automatic idle shutdown yet. Stop/restart a background backend after rebuilding
its executable to ensure you're running the new code.

## API

The `waypoints-backend` library provides the shared configuration, client,
connect-or-start logic, and server. A future GUI can use the same client while
launching the companion `waypoints` executable; the GUI does not call CLI workflow
commands or access the database.

Transport: a local Unix socket, one newline-delimited JSON-RPC 2.0 request per
connection. Clients reconnect for each request. The backend serializes requests.
Frames are limited to 1 MiB. The server's frame reads/response writes have
five-second timeouts. Clients allow five seconds for ordinary requests and
120 seconds for `node.workspace.create`.
Storage mutations are not cancelled by a server-side transport timeout. Requests
are not automatically retried: a lost response may follow a successful mutation.

Methods:

| Method | Parameters | Result |
|---|---|---|
| `backend.info` | `{}` | `{pid, database, protocol_version}` |
| `backend.shutdown` | `{}` | `null` |
| `workflow.create` | `{project_path, name}` | `{id, project_path, name}` |
| `workflow.list` | `{project_path}` | Array of workflows ordered by ID |
| `workflow.graph` | `{workflow_id}` | `{workflow, nodes, connections}` |
| `node.create` | `{workflow_id, prompt}` | `{id, workflow_id, prompt}` |
| `node.get` | `{workflow_id, node_id}` | `{id, workflow_id, prompt}` |
| `node.update` | `{workflow_id, node_id, prompt}` | Updated node, with the same ID |
| `node.associate` | `{workflow_id, node_id, change_id}` | `{node, association, state}` with the selected current revision |
| `node.create_change` | `{workflow_id, node_id, base}` | `{node, association, state}` for the newly created empty change |
| `node.workspace.create` | `{workflow_id, node_id, directory}` | `{node, workspace, state}` for a new or already-saved workspace |
| `node.workspace.status` | `{workflow_id, node_id}` | `{node, workspace, state}` observed without updating files |
| `node.refresh` | `{workflow_id, node_id}` | `{node, association, state}` with freshly queried native status |
| `connection.create` | `{workflow_id, source_id, target_id}` | `{workflow_id, source_id, target_id}` |

Example:

```json
{"jsonrpc":"2.0","id":1,"method":"workflow.list","params":{"project_path":"/absolute/project/path"}}
```

Syntax errors, invalid requests, unknown methods, and invalid parameter shapes use
JSON-RPC codes -32700, -32600, -32601, and -32602. Storage/domain failures use
-32000 with a descriptive message. Notifications receive no response. Batch
requests and subscriptions are not supported. Oversized or incomplete frames
close the connection.

## Checks

```sh
cargo test -p waypoints-core -p waypoints-backend -p waypoints-cli
cargo clippy -p waypoints-core -p waypoints-backend -p waypoints-cli --all-targets --all-features -- -D warnings
cargo doc -p waypoints-core -p waypoints-backend -p waypoints-cli --no-deps
cargo fmt --check
```

End-to-end tests launch the real executable and background processes with isolated
temporary databases, then stop the backend during cleanup.
Native integration tests require `jj` and create disposable repositories. They
exercise rewrites, abandonment, divergence, conflicts, and unchanged operation IDs
when querying dirty working copies.

## Deliberately not included

- Windows named pipes (the backend currently requires macOS/Linux).
- GUI integration, notifications, or durable event replay.
- Project registration, undo history, jj mutations, or ACP.
- General versioned database migrations beyond additive create-if-missing tables.
