# Restore a saved result within the same change

## Meaning

A node owns one evolving jj change. Each successful run can record an exact
revision of that change: a different commit ID, with the same change ID.
Restoring an older result does **not** create another node or a child change.

Waypoints first snapshots the current jj-tracked files, then runs native
`jj restore --from SAVED_COMMIT --into @` in the node's dedicated workspace.
The result is another revision of the **current change**, containing the selected
saved tree. Current parents and description stay unchanged. This is not
`jj edit SAVED_COMMIT`, which can make an old hidden revision visible and create
divergence. Native evolution retains the later revisions instead of replacing
them with an application-owned snapshot archive.

## CLI

```sh
waypoints run list --workflow 1 --node 2
waypoints run review --run 3
waypoints run restore --run 3 --json

# Optionally change the assignment, then explicitly start a fresh session.
waypoints node update --workflow 1 --node 2 --prompt "Try a simpler login design"
waypoints run start --workflow 1 --node 2 --executable /absolute/path/to/codex-acp
```

Use actual IDs from your workflow. `restore` changes code but does not launch an
agent or restore prompt text. `run start` uses the node's current saved prompt.
The node, change association, graph edges, metadata Undo/Redo, and all old run
records remain unchanged. The original project checkout is not snapshotted or
switched; only the dedicated editing workspace changes.

The response is a `RestoreOperation` with:

- `id`, selected `run_id`, `workflow_id`, `node_id`, and exact `workspace`.
- `source`: selected saved revision.
- `before`: recorded current revision before snapshotting local edits.
- `preserved`: checkpoint including current jj-tracked edits, saved **before**
  restoration starts.
- `after`: verified restored revision.
- `native_operation`: native operation used for preflight.
- `session_barrier`: last run ID before reservation.
- `state`: `unresolved`, `completed`, or `recovered`.

The checkpoint follows native tracking/ignore rules. Ignored files are not a
backup, and restoring a tree is not filesystem cleanup. Snapshot diagnostics
stop the operation before restoration; restore diagnostics leave an unresolved
operation for inspection. Stored commit IDs are not garbage-collection pins.
Checkpoint commits can be inspected with native jj even though they are not
invented agent runs or entries in `run list`.

## First-version safeguards

Before reserving or changing code, restoration requires:

- No active or interrupted agent runs **anywhere in this backend database**.
  This conservative rule also avoids competing background finalization.
- No unresolved dependency edit or earlier restore.
- A saved native revision on a live node, still matching its allocated workspace
  and current change ID.
- A ready, freshly verified physical checkout, one current mutable revision,
  and no other workspace editing that change.
- No saved outgoing edges, including prompt-only downstream tasks, and no native
  descendants, including work outside the saved workflow.
- Exactly the same parent commit IDs on the saved and current revision.
  Restoring across a reparent/upstream rewrite is not supported.

Stop external tools and jj writers before restoring. Preflight rechecks native
operations and checkout identity, but Waypoints does not lock out unrelated
external processes or sandbox the filesystem.

Restoration invalidates continuation of **all pre-existing sessions on this
node**, not merely the selected run's session. Old conversation records are
retained, not rewound. `run continue` refuses them and asks for a fresh run.
New sessions created after restoration can continue normally. Even an uncertain
restore keeps this conservative session barrier.

## Uncertainty and explicit recovery

The backend reserves a journal entry transactionally **before** snapshotting,
then checkpoints `preserved` before invoking restore. An interrupted command,
diagnostic, timeout, or database failure can follow a successful native mutation.
There is no automatic retry, rollback, guard clearing, or startup repair.

```sh
waypoints run restore-status --run 3 --json
# Inspect the recorded commit IDs and the dedicated workspace with native jj.
# Stop all old tools/native writers and manually reconcile any stale checkout.
waypoints run recover-restore --operation 1 --confirm-processes-stopped --json
```

`restore-status` lists attempts for the selected run, including their operation
IDs, without calling jj or modifying files. Restore and status can auto-start the
backend. Recovery confirmation requires a running backend.

While unresolved, the restore blocks the same mutation entry points as an
unresolved native dependency edit, including metadata edits and agent starts.
Read-only inspection remains available. If a response was lost or timed out,
inspect status rather than repeating `restore`.

Recovery verifies the saved workspace/change, fresh checkout, leaf constraints,
and unchanged parents. It never repeats restore, snapshots files, signals old
PIDs, or claims current code was produced by the old run. It records the observed
revision as `after`, sets `recovered`, and releases the guard. This is an explicit
acknowledgement of reconciled native state, **not proof restoration succeeded**.
Repeated recovery of an already resolved operation returns its saved record.
Missing/divergent changes and stale/mismatched checkouts remain refused.

## API and persistence

Core and backend client expose `restore_run(run_id)`, `run_restores(run_id)`,
and `recover_restore(operation_id, confirmation)`. Core also exposes
`restore_operation(id)` for an individual journal record.

| RPC method | Parameters | Result |
|---|---|---|
| `run.restore` | `{run_id}` | `RestoreOperation` |
| `run.restore_status` | `{run_id}` | Ordered array of attempts |
| `run.recover_restore` | `{operation_id, confirmation?}` | Resolved `RestoreOperation` |

Recovery requires `confirmation: "processes_stopped"`; omitted/null is a refusal.
Unknown fields and wrong types are invalid parameters, not force options.
Restore/recovery have 120-second client deadlines and a 110-second core budget.
Requests remain serialized; long restoration requests may delay other controls.

Opening an existing database additively creates `restore_operations` and a unique
active-operation index. Reservation, real journal ID, and session barrier are
committed together. No existing run, graph, or metadata-history format changes.
Protocol version 1 is retained; restart an older backend after rebuilding.

## Validation and remaining scope

Automated disposable native-jj/core and CLI/backend/fake-ACP tests cover:

- Same node/change identity, exact selected code, retained parent IDs, original
  checkout, graph/history, and exact historical run review.
- Pre-restore dirty/new tracked files available at the preserved commit, with
  later results still in native evolution.
- Restart persistence, rejection of old sessions, and fresh agent execution
  followed by valid continuation of the fresh session.
- Busy runs, absent snapshots, saved/native descendants, stale checkouts,
  changed parents, and strict RPC parameters.
- Transaction rollback after the journal INSERT when the real-ID write fails.
- Failures before snapshot and after a successful native restore; persistent
  guards, no blind replay, stale-checkout recovery refusal, and explicit release.

Native [revision browsing and exact previews](22-native-revision-history.md) are
available separately. Restoration from arbitrary commit IDs/checkpoints,
downstream rebasing, historical prompt or conversation replay, and indefinite
native-object retention remain outside this restoration slice.

### Manual Codex acceptance

The real CLI/backend/installed Codex adapter path passed a disposable three-run
test using jj 0.46.0, codex-acp 2.5.0, and Python 3.15.1:

1. Run A created `app.py` with a static HTML login form.
2. Run B continued A's session and added `@keyframes login-enter` and an entrance
   animation. Both results were saved on the same node/change.
3. After a backend restart, `run restore --run 1` restored A's `app.py`
   byte-for-byte. Graph, metadata history, node identity, and change ID stayed
   unchanged. Both old-session continuation attempts were refused.
4. The node prompt was explicitly updated; run C started a different session and
   produced a simpler static design headed “Welcome back,” without animations.
5. Exact reviews of A, B, and C remained available after another backend restart.
   All three commit IDs remained in native evolution; viewing them did not
   advance the native operation head.

All three agent runs completed with saved snapshots. Each ran the exact command
`python3 -B -m unittest discover -s tests -v`, with four tests passing and exit 0
observed in **live** ACP tool events. The same command independently passed after
A, B, restoration of A, and C. Checks covered the HTML document/form, email and
password inputs, and submit button, not browser rendering or visual design.
The saved ACP event tails reported `gap: true`; they are bounded evidence, not
complete transcripts.

One node and change ID, `mpvvqzpztprmzompxllxvuknyynvsuxs`, owned every result:

| Result | Run | Commit ID |
|---|---|---|
| A: static form | 1 | `36e22b9b63a6fa5cc135fe1b1f72ec9ce73ee66e` |
| B: animated form | 2 | `21de2382520220fbaccbcc677aba1e45c50533a7` |
| C: simpler form after restoring A | 3 | `24d5a830cb165f067f33c3fd513d9d366898706f` |

A/B shared session `01a198eb-b6ba-7402-8b97-9b2dac00fac1`; C used fresh session
`01a198ec-b991-7160-9325-cf8f0aaea2bc` with no `continued_from`.
Every saved result retained the exact fixed base parent
`d375bbbd353bf0337061cd5979e0cb5afb6be03d` and changed only `app.py`.
README/tests and the original project's checkout/files remained unchanged.

Disposable database, project, and JSON evidence are under the ignored
`target/restore-acceptance/` directory. The editing workspace used turn-local
scratch outside existing jj workspaces and is not a persistent allocation to
reuse. Commands inherited stable `TMPDIR=/tmp`, avoiding the previously observed
short-lived terminal-temp-directory issue. Existing installed authentication was
used without copying credentials; no model was explicitly selected or recorded.
The disposable backend was stopped and socket removal confirmed.

This is manual acceptance of the clean leaf restoration flow, not downstream
rebasing, arbitrary-history navigation, browser acceptance, or crash recovery.
Crash/uncertainty safeguards retain their separate automated coverage.
