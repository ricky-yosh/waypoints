# Browse native revision history

The core/backend/CLI can list a node's native jj evolution and preview an exact
commit without snapshotting, switching checkouts, creating nodes, or launching an
agent. This is the data layer for a future visual history picker, not that UI.

## CLI

```sh
waypoints node revisions --workflow 1 --node 2 --page-size 20 --json
# Pass the returned opaque next_cursor exactly, quoted as one argument:
waypoints node revisions --workflow 1 --node 2 --page-size 20 --cursor "$cursor" --json

waypoints node review --workflow 1 --node 2 --commit FULL_COMMIT_ID
```

Without `--json`, the listing prints full commit IDs, current/historical markers,
native operation descriptions/timestamps, matching saved-run IDs, and a next
cursor or end marker. Labels are native data, not invented friendly card names.
`node review` without `--commit` retains its existing current-revision behavior.

The exact preview selector must be a full, 40-character lowercase hexadecimal
commit ID. Prefixes, change IDs, bookmarks and arbitrary revsets are refused.
The existing review response is `{revision, parent_commit_ids, summary, patch}`;
its diff compares against the code-parent tree, **not** the previous evolution
entry. It is not a full-file-content endpoint or an evolution inter-diff.

Preview is scoped to the associated repository, not a proof of membership in the
current page. It accepts related-change commits, including predecessors exposed
by squash, and other exact commits in that repository. Missing objects are errors.
Preview does not restore code. Restoration remains the separate
[`run restore`](21-saved-result-restoration.md) action for eligible saved runs.
Restoring an arbitrary manual revision by commit ID is not implemented here.

## Response and native meaning

`NodeRevisionPage` contains:

- `workflow_id`, `node_id`, and the associated `change_id`.
- `native_operation`: the repository operation pinned for this traversal.
- `current_commit_ids`: **all** visible revisions of this change at that operation.
  More than one means divergence; no current revision is silently chosen.
- `entries`, each containing:
  - `evolution.revision`: exact change/commit IDs, description and conflict flag.
  - `evolution.predecessor_commit_ids`: native rewrite/evolution links.
  - `evolution.operation_id`, `operation_description`, and `timestamp`.
  - `is_current`: whether this commit is one of those pinned visible revisions.
  - `run_ids`: this node's runs whose saved snapshots match the exact commit.
- `next_cursor`: an opaque string, or null at the end of available traversal.

Predecessors describe **evolution**, not code ancestry. The entries use native
topological order: newer versions before their predecessors, not a promise of
strict timestamp order. Branches/convergence are retained as links. Squash can
include a different change ID; clients must not relabel those as revisions of the
node's own change. Manual entries without saved agent runs have empty `run_ids`.
Several runs may match the same revision; no fake run is invented for manual work.

No editing workspace is required. Unallocated, stale or absent editing folders
do not stop browsing when the original project repository and native objects are
available. An unassociated node or a fresh traversal with no visible revision is
an error, not an empty history assertion. Existing exact saved-run review remains
available independently while its objects are retained.

## Pagination, changes and limits

CLI defaults to 20 entries; core/RPC require a page size between 1 and 100.
There is no application-imposed maximum revision count.

Each traversal pins a native operation and starts from every visible head.
Its cursor carries the remaining predecessor frontier, rather than an ever-growing
list of all visited commits or an offset that re-emits earlier pages. Native
topological ordering lets converging predecessors appear once. Changing page size
between requests is allowed.

Later pages keep their original `current_commit_ids` and `is_current` meaning,
even if the live change advances or is abandoned. Omit the cursor to refresh.
Saved-run annotations are read from current database metadata, not frozen at that
native operation. Restarting the backend does not invalidate a cursor.

Cursors bind node/workflow, association, repository and native operation. Invalid
versions, malformed data, another node/association, changed roots or unavailable
native operations are errors. Cursors are opaque traversal state, **not signed
authorization tokens or retention pins**; do not edit them.

Each complete listing/preview has a four-second budget within the ordinary
five-second client deadline. Native subprocesses retain their two-second timeout
and 1 MiB per-stream output bound. Pages have a 900 KiB JSON limit and cursors a
64 KiB limit. Very large descriptions, wide frontiers or slow native history can
therefore return an actionable error; output is never silently truncated.
Page size bounds returned entries, not jj's internal topological traversal cost.
Use a smaller page or native jj when these bounds are exceeded.

Native garbage collection and operation-history retention still apply. Waypoints
does not keep an independent revision archive or guarantee permanent availability.
Native diagnostics or unavailable objects/operations fail the request rather
than being presented as a successfully complete empty history.

## API and safety

Core and backend client expose `node_revisions(workflow_id, node_id, page_size,
cursor)` and `review_node_revision(workflow_id, node_id, commit_id)`.

| RPC | Parameters | Result |
|---|---|---|
| `node.revisions` | `{workflow_id, node_id, page_size, cursor?}` | `NodeRevisionPage` |
| `node.revision_review` | `{workflow_id, node_id, commit_id}` | `RevisionReview` |

Unknown fields and wrong types are invalid parameters; invalid bounds, cursors
and selectors are domain errors. `cursor: null` starts a fresh traversal.
Both commands may auto-start the backend. Existing `node.review` RPC parameters
remain unchanged. Restart an older backend after rebuilding; protocol version 1
is retained for the additive methods.

Reads use native `--ignore-working-copy` and pinned `--at-op`, with no Git import,
concurrent-operation integration, checkout repair, workspace allocation,
graph/history mutation or agent-session change. Run annotations read saved
snapshot IDs from existing run records. No schema migration, snapshot archive or
new persistent history table is introduced.

## Automated validation

Disposable native-jj and CLI/backend/fake-ACP tests cover:

- Multi-page manual evolution, operation-pinned traversal across external
  rewrites and backend/storage restarts, explicit refresh, and exact previews.
- All divergent visible heads, branched/converging links without duplicates,
  and squash history containing related change IDs.
- Conflict flags, dirty checkouts, unchanged native operations and metadata.
- Saved-run annotations, human/JSON CLI paths and strict RPC validation.
- Invalid bounds/cursors/commit selectors, unassociated and abandoned changes,
  and continued historical browsing at an already pinned operation.
- Oversized native history refusing the request instead of returning a partial
  page; a smaller page still succeeds.

The visual picker, arbitrary-commit restoration and native-object retention
policy remain separate work. This feature does not expand restore eligibility,
resolve divergence or rewind agent conversation history.

### Retained real-agent history smoke check

The built CLI also browsed the retained disposable
[Codex restoration fixture](21-saved-result-restoration.md#manual-codex-acceptance):
five native revisions across three two-entry pages. A/B/C saved-run annotations
matched their original runs, and exact-commit previews matched the previously
recorded reviews. The native operation head was unchanged. Evidence is in the
ignored `target/restore-acceptance/revision-browser-smoke.json`; the disposable
backend was stopped and socket removal confirmed. No new agent run or visual
picker/browser test was performed for this check.
