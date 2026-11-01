# Explicit workflow starting points

## Implemented scope

A workflow can save an explicit native starting point, usually the local `main`
bookmark. Users may instead select another local bookmark or a native commit/change
ID. Selection resolves once to an exact commit and change ID.

**Starting from main means main as recorded now, not following main forever.**
No fetch, snapshot, checkout, bookmark move, rebase, agent launch, or native history
mutation occurs. A later explicit update-base operation is not implemented.

This is the first foundation for the
[rooted dependency model](15-native-dependency-editing.md), not its completion.
Selecting a base does not yet create root edges, validate node reachability,
reconcile existing node ancestry, or restrict node/connection creation.

## CLI

From an existing jj project, explicitly choose a starting bookmark:

```sh
waypoints workflow create --project . --name "Authentication" --base main --json
waypoints workflow create --project . --name "Follow-up" --base feature/auth --json
```

Or choose a native revision:

```sh
waypoints workflow create --project . --name "Exact starting point" \
  --base-revision COMMIT_ID --json
```

`--base` and `--base-revision` are mutually exclusive. Bookmark input is an exact
**local** name, not a glob, tag, remote bookmark such as `main@origin`, or revset.
The revision option accepts a native commit/change ID or unique prefix, plus the
explicit virtual `root()`. Arbitrary revsets such as `@` and `all()` are refused.
Missing, ambiguous, divergent, or conflicted bookmark targets are errors; there is
no fallback to a different branch. Code conflict status on a single selected
revision is recorded, not silently resolved.

Inspect the saved value (use the actual returned workflow ID):

```sh
waypoints workflow get --workflow 1 --json
waypoints workflow list --project . --json
waypoints workflow graph --workflow 1 --json
```

All expose the same saved `workflow.base`:

```json
{
  "repository_path": "/path/to/project",
  "source": {"kind": "bookmark", "name": "main"},
  "revision": {
    "change_id": "FULL_CHANGE_ID",
    "commit_id": "FULL_COMMIT_ID",
    "description": "Description at selection time",
    "conflicted": false
  }
}
```

For a revision selection, `source` is `{"kind":"revision","id":"USER_SELECTION"}`.
The exact resolved identity is always in `revision`, regardless of the selector.
Inspection reads saved data only: it does not resolve the bookmark again or require
jj. Saved native revision references do not override jj garbage-collection rules;
this feature does not pin history with native bookmarks.

## Compatibility and existing workflows

For this additive slice, **omitting both base flags preserves the existing
unbased behavior**. Such workflows return `"base": null` and can still be created
without jj. No implicit `main` is assigned to old data or old clients.
Defaulting/requiring a base will be addressed with rooted-connectivity migration;
for now, use `--base main` explicitly for the normal code-workflow path.

Assign a starting point to an existing unbased workflow:

```sh
waypoints workflow set-base --workflow 1 --base main --json
```

This command requires exactly one selector and refuses to replace any existing
base, including the same bookmark name after it advances. A repeated/lost-response
attempt should inspect `workflow get`, not assume reselection is safe.

The command only saves the initial base reference. It does **not** claim that
existing nodes are connected to it, change their code, clear metadata undo history,
or rewind agent sessions. It is not an adoption/rebase/update-base shortcut.

Rebuild and stop an older backend before using these additive operations.

## Backend and persistence

- The `workflow_bases` table has one serialized base record per workflow.
  Existing workflow/node tables and records are preserved.
- Creating a workflow and saving its base share one transaction. Failed resolution
  creates no workflow; a failed base write rolls back the workflow insert.
- Initial base selection is insert-only, with a unique workflow key preventing
  concurrent replacement.
- Native selection uses read-only `--ignore-working-copy --at-op=@` queries.
  A four-second total native budget fits the ordinary five-second client deadline;
  each query retains existing two-second/output bounds. Requests remain serialized.
- No undo-boundary acknowledgement is required here: saving a base does not change code.

| RPC method | Parameters | Result |
|---|---|---|
| `workflow.create` | `{project_path, name, base?}` | Workflow including nullable `base` |
| `workflow.get` | `{workflow_id}` | Saved workflow including nullable `base` |
| `workflow.set_base` | `{workflow_id, source}` | Workflow with its initial base saved |

`base`/`source` use the tagged selector shapes shown above, not arbitrary strings
or raw revsets. Extra fields and malformed selectors are rejected. Missing `base`
on creation retains compatibility with unbased workflows. Lists and graphs expose
the same nullable base field. Protocol version 1 is retained for this additive API;
older servers reject the new fields/methods rather than perform native selection.

## Validation

Native/core and real CLI/backend tests cover:

- Main and alternate local bookmarks, commit IDs/prefixes, and change IDs.
- Bookmark movement plus database/backend restart preserving the exact base.
- Unchanged native operation IDs, dirty files, and checkout metadata after selection.
- Missing/conflicted bookmarks, literal glob/revset-looking names, invalid revisions,
  malformed RPC selectors, and mutually exclusive/missing CLI arguments.
- Initial selection for legacy workflows preserving nodes, edges, and undo history.
- Refusal to replace an existing base.
- Additive old-database upgrade with no invented base.
- A base-write SQL failure rolling back a new workflow.

Tests use disposable repositories and databases. No model calls or GUI are needed.
