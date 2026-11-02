# Confirmed restore with downstream work

Restore an exact revision into its existing change and let native jj rebase its
descendants. This is code restoration, **not rerunning downstream tasks or
rewinding agent memory**. Existing nodes, change IDs, graph edges, metadata
Undo/Redo and historical run records remain unchanged.

## CLI

```sh
# Use actual IDs and a full same-change lowercase commit ID.
preview=$(mktemp)
waypoints node restore-impact --workflow 1 --node 2 --commit FULL_COMMIT_ID --json > "$preview"
# Inspect the report before explicitly accepting its effects.
cat "$preview"
waypoints node restore-with-descendants --preview "$preview" --confirm-restore-and-rebase --json
waypoints node restores --workflow 1 --node 2 --json
```

The preview contains task prompts and session references; keep it private rather
than committing it to the project. The CLI reads it as JSON, not as shell input.
The apply command can auto-start the backend. Missing confirmation is a CLI
syntax error. There is no `force` option, automatic retry, or implicit refresh
that silently accepts a different impact report.

Core and backend client expose
`restore_node_with_descendants(preview, confirmation)`. JSON-RPC method
`node.restore_with_descendants` takes `preview` as the complete `RestoreImpact`
JSON object and `confirmation` as `"restore_and_rebase"`.
The response is a `RestoreOperation`.
Unknown parameter fields and invalid confirmation values are rejected.
Client deadline is 120 seconds, with a 110-second core budget. This owns the
serialized storage lane; live run controls bypass it, while other requests
receive the [not-started busy response](25-backend-frontend-readiness.md#scheduling-and-run-controls).

## Narrow supported scope

Application recomputes and compares the **entire** preview, including the native
operation, selected/current revisions, source-workflow graph and edges, tasks,
workspace observations, session references, active reservations and restore
journal watermark. A change means refresh, inspect and confirm again.
The watermark also prevents reusing a preview after a native no-op restore.

Before reservation, require:

- A rooted based workflow, one current mutable source change, and a selected
  revision of that same change with the same exact code parents.
- Exactly this workflow's transitive saved subtree. Every associated affected
  node has one unique current mutable revision with matching native code
  parents. Every native descendant has one saved owner in that subtree.
  Graph-only prompt tasks are included, without inventing native changes.
- No unmanaged, cross-workflow or divergent descendants. Native merges within
  the subtree are supported; their other saved inputs can remain outside the
  subtree, but must stay unchanged.
- A ready, fresh dedicated source workspace. A descendant need not allocate a
  workspace, but every existing allocation must be ready and physically fresh.
  Additional/unmanaged editing workspaces are refused.
- No active/interrupted runs anywhere in the database, unresolved native
  dependency/restore operations, or incomplete native allocations.
- A completion record that fits the response bound; oversized scope is refused
  before any checkpoint or journal reservation.

These checks do not lock external jj writers, configuration files or filesystem
edits. **Stop outside writers first.** The preview compares recorded facts, not
unsnapshotted file bytes. Confirmation accepts checkpointing the tracked edits
present at application time, not a promise that files were immutable since
preview. Native operations are checked before and after steps; unexpected
changes retain the guard rather than claiming completion.

## Preservation and native execution

1. Transactionally compare the saved graph and confirmed journal watermark,
   reserve the restore, and capture a database-wide run-ID session barrier.
2. Walk affected tasks **leaves first**. Snapshot each allocated native workspace
   and persist its exact checkpoint before proceeding toward ancestors. For
   unallocated native changes, retain their exact recorded revision. Prompt-only
   tasks have no invented code revision.
3. Snapshot the source last. Ancestor snapshots can rebase descendants, but their
   local edits have already been checkpointed. Verify the same native/saved
   scope, unchanged boundary inputs and workspace ownership again.
4. Use ordinary native `jj restore` in the source workspace. It preserves current
   source identity, parents and description; jj rebases descendants.
5. Verify every affected change's identity and native parents, exact descendant
   set, unchanged graph/boundary inputs, and workspace ownership. Persist the
   observed results before releasing the guard.

Source files are updated to the selected tree. **Descendant physical checkouts
are not updated.** `needs_workspace_update` identifies allocations whose physical
metadata differs from the observed native result. Inspect them and explicitly
use native `jj workspace update-stale` when appropriate; this can materialize
conflict markers. Waypoints does not perform that repair automatically.

Successful native rebases can contain conflicts. These are completed code
outcomes with `after.conflicted: true`, not hidden resolutions or agent failures.
Snapshot/native diagnostics, failed verification or uncertain writes retain an
unresolved operation instead.

Native tracking/ignore rules still apply. Ignored files are not backed up.
Checkpoints are jj revisions, not application-owned file archives or GC pins.
A descendant checkpoint can be inspected even when it cannot be restored with
the ordinary same-parent restore command after its upstream parent changed.

## Results and agent context

`RestoreOperation.downstream` is absent on old/ordinary leaf restores. New
checked operations include:

- The confirmed `preview_restore_watermark` and unchanged `graph`.
- `nodes` in leaf-first order: `node_id`, optional `before`, original
  `before_parent_commit_ids`, optional `workspace`, `preserved`, observed `after`,
  and `needs_workspace_update`.
- `context_notice`: rebased code was **not rerun by agents**, and all old
  affected sessions require a fresh start, even for clean rebases.

In an unresolved operation, `after` observations may describe a checkpoint-stage
state, not a successful restore. Checkout flags likewise describe the last
observation, not a live status query after subsequent manual updates.
`Completed` means verified native restoration;
`Recovered` only acknowledges manually reconciled state. Missing checkpoints
mean preservation was not durably confirmed; never assume a failed call did
nothing.

The source and **every affected task** receive the session barrier at reservation,
including uncertain attempts. Old `run continue` calls refuse with a fresh-run
instruction. After explicit checkout reconciliation, explicitly start a fresh
run with the desired saved assignment. Nothing launches, resumes, injects
context or reruns tasks automatically. A new session can continue normally.
This is not a general causal-staleness tracker or human review gate.

`node restores` on any live affected task returns the same source operation,
so descendants do not hide the event. `run restore-status` retains its existing
run-selected-only semantics. Commit-selected application has `run_id: null`;
no fake agent runs are created.

## Interruption and explicit recovery

Inspect `node restores` first. Never blindly repeat apply after timeout, lost
response, diagnostic or crash. All normal mutations remain globally guarded.
Stop old/native writers, inspect the recorded checkpoints and manually reconcile
**every allocated affected checkout** before recovery:

```sh
waypoints run recover-restore --operation OPERATION_ID --confirm-processes-stopped --json
```

Recovery verifies the original saved graph, identities, native subtree,
workspace allocations/ownership, fresh physical checkouts and unchanged boundary
parents. It never snapshots, restores, rebases, signals old processes or updates
files. It records observed revisions as `Recovered` and releases the guard;
it does **not** claim restoration succeeded. Old-session barriers remain.
Repeated recovery of a resolved operation returns its recorded result.

The journal uses the existing restore table and an optional JSON extension.
Legacy records remain readable and omit the new field when serialized. No new
snapshot archive or SQL migration is needed for this extension. **Do not run an
older backend against a database containing downstream restore records**:
older code does not understand descendant session barriers. Protocol version 1
is additive; rebuild/restart the backend before using the new command.

## Validation and remaining limits

Automated native-jj tests cover dirty source/descendant checkpoints, unchanged
graph/history/run records, clean/conflicted rebases, merge boundary inputs,
explicit fresh sessions, unsafe/native-divergent/cross-workflow/unmanaged work,
stale previews and checkouts, oversized completion refusal, and no-op preview
replay prevention. CLI/fake-ACP tests cover strict confirmation/RPC, backend
restart, old-session refusal and explicit fresh-session launch.

Real backend crash tests cover before the first snapshot, after a descendant
checkpoint, and after native restore; recovery never repeats the mutation.
A journal-write fault test preserves checkpoints, global reservation and child
session barriers across restart when completion cannot be saved.

Broader cross-workflow/unmanaged reconciliation, cross-change/changed-parent
restoration, automatic conflict resolution, automatic workspace repair and
review policy remain outside this slice. General inferred-context-staleness
tracking is outside [v1 scope](02-architecture.md#decision-reflect-native-jj-changes-not-inferred-context-staleness);
the restore-specific session barriers remain enforced.

### Manual CLI acceptance

`target/downstream.xqmv39nh/acceptance.json` records a passing built-CLI
`Build API → Build login → Test login` flow with two native changes, two
allocated workspaces and a final prompt-only task.

After recording API A and B, API C and login edits were left unsaved. Confirmed
application restored API A into the same source change, checkpointed both dirty
workspaces, rebased the login change without changing its identity, and flagged
its untouched physical checkout. Exact checkpoint contents were inspected with
native jj. Graph/history and the original project checkout stayed unchanged.

Reusing the old preview was refused without another native operation. The
descendant's journal matched after backend restart. Only an explicit manual
`jj workspace update-stale` updated its API files; its login edits survived.
No agent run was created. The backend was stopped afterwards.

Editing workspace paths in this evidence were turn-local scratch directories,
not durable/reusable workspaces. Agent-session behavior is covered by fake-ACP
tests; this initial smoke did not run an agent. The subsequent real-Codex
acceptance below covers the integrated session behavior.

### Manual real-Codex acceptance

`target/real-downstream-acceptance/acceptance.json` records a passing built-CLI
and backend flow with jj 0.46.0, codex-acp 2.5.0 and Python 3.15.1. All work
used a new disposable project and the installed adapter's existing authentication;
no credentials were copied into the fixture.

The saved graph was `Build API → Build login → Test login`, with two native
changes and dedicated editing workspaces; the last task remained prompt-only.
Four real Codex turns completed with saved exact revisions:

| Run | Work | Session behavior |
|---|---|---|
| 1 | API A: username validation and profile lookup | Fresh API session |
| 2 | API B: add a profile role | Explicit continuation of run 1's session |
| 3 | Login consuming the API's username field | Fresh downstream session |
| 4 | Validate login against restored API A and add a regression test | Explicit fresh downstream session, distinct ID and no `continued_from` |

Before restoration, separate manual edits were left unsnapshotted in the source
and login workspaces. The preview identified both changes, all three tasks, both
workspaces and both sessions, without changing files or the native operation.
Explicit application then:

- Preserved both manual edits in exact native checkpoints and restored the
  source's complete file tree to API A.
- Cleanly rebased login, retaining both change IDs. Its physical files stayed
  byte-for-byte unchanged and `needs_workspace_update` was true.
- Left the original project checkout, saved graph, metadata history and all
  historical run records unchanged. No agent launched automatically.
- Refused reuse of the old preview, both old sessions, and a fresh run against
  the stale login checkout, without another native operation.

After backend restart, the descendant journal matched and its old session was
still barred. An explicit native `jj workspace update-stale` brought API A into
login's checkout while retaining the preserved login edit. Only then was the
login assignment explicitly updated and run 4 started. It inspected the current
files, added only the requested regression test and preserved all other files.

Independent `python3 -B -m unittest discover -s tests -v` checks passed after API A
(6 tests), API B (7), login (11), restored API A (6), manual login checkout update
(10), and the fresh session (11). Original and new saved-run reviews, native
revision browsing, historical run records and the journal visible from the
prompt-only task were also checked. A further backend restart retained the run
records and journal; the backend was stopped and its socket absent at the end.

The evidence directory contains the driver, preview, restore operation, exact
checkpoint excerpts, refusal diagnostics, independent test output, run records,
reviews, revision pages and bounded live-event polls. A poll gap occurred during
API continuation; these are not a complete transcript archive. Editing workspace
paths are turn-local scratch directories, not durable/reusable workspaces.

This acceptance exercised a **clean rebase**, not a conflict or process crash.
Conflicted outcomes and interrupted recovery retain the automated coverage
described above. No GUI/browser acceptance was performed, and this does not
establish general context-staleness tracking or broader affected-work repair.
