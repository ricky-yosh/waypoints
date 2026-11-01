# Restore saved code within the same change

## Meaning

A node owns one evolving jj change. Each successful run can record an exact
revision of that change: a different commit ID, with the same change ID.
Manual revisions and pre-restore checkpoints can also be selected by exact
commit ID, without an agent run. Restoration does **not** create another node or
a child change.

Waypoints first snapshots the current jj-tracked files, then runs native
`jj restore --from SELECTED_COMMIT --into @` in the node's dedicated workspace.
The result is another revision of the **current change**, containing the selected
saved tree. Current parents and description stay unchanged. This is not
`jj edit SELECTED_COMMIT`, which can make an old hidden revision visible and create
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

### Select an exact native commit

Before deciding to restore, use [readonly restore impact](23-restore-impact.md)
to inspect potentially affected changes, tasks, workspaces and sessions.
`node restore-impact` does not change code or simulate conflicts.
For checked same-workflow downstream application, explicitly confirm its saved
report using [restore with descendants](24-downstream-restoration.md). The
ordinary `run restore` and `node restore` commands below retain their leaf safeguards.

```sh
waypoints node revisions --workflow 1 --node 2 --json
waypoints node review --workflow 1 --node 2 --commit FULL_COMMIT_ID
waypoints node restore --workflow 1 --node 2 --commit FULL_COMMIT_ID --json
waypoints node restores --workflow 1 --node 2 --json
```

Use the full 40-character lowercase hexadecimal commit ID. Prefixes, change IDs,
bookmarks and revsets are refused. The commit must be available in this
repository, have the node's own change ID, and have exactly the same parent
commit IDs as its current revision. It need not have an agent run, appear on the
latest history page, or still be a visible version of the change.

This also permits selecting an earlier operation's `preserved.commit_id` to
restore the edits saved before that operation. No fake agent run is created.
Related-change squash history can be previewed, but cross-change restoration is
refused before reservation or snapshotting. Missing/collected commits, divergent
current changes and all existing workspace/downstream safeguards remain refused.

`node restores` lists **all** attempts for the live node, whether selected by
run or commit ID. `run restore-status` still lists only attempts explicitly
selected through that run; selecting its commit directly does not invent run
provenance. Both forms share the same native execution, checkpointing, session
barrier, unresolved guard and recovery.

The response is a `RestoreOperation` with:

- `id`, `workflow_id`, `node_id`, and exact `workspace`.
- `run_id`: selected saved run, or null for a direct commit selection.
- `source`: selected exact native revision.
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
Checkpoint commits can be inspected and selected for restoration even though
they are not invented agent runs or entries in `run list`.

## First-version safeguards

Before reserving or changing code, restoration requires:

- No active or interrupted agent runs **anywhere in this backend database**.
  This conservative rule also avoids competing background finalization.
- No unresolved dependency edit or earlier restore.
- An available native revision on a live node, matching its current change ID.
  Run-based selection additionally requires its original saved workspace and
  snapshot identity to match.
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
# For all attempts, including direct commit selections:
waypoints node restores --workflow 1 --node 2 --json
# Inspect the recorded commit IDs and the dedicated workspace with native jj.
# Stop all old tools/native writers and manually reconcile any stale checkout.
waypoints run recover-restore --operation 1 --confirm-processes-stopped --json
```

Both status commands include operation IDs without calling jj or modifying
files. Restore and status can auto-start the backend. Recovery confirmation
requires a running backend. The existing `run recover-restore` command handles
both kinds of operation; it does not require an associated agent run.

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
`restore_node_revision(workflow_id, node_id, commit_id)`,
`node_restores(workflow_id, node_id)`, and
`recover_restore(operation_id, confirmation)`. Core also exposes
`restore_operation(id)` for an individual journal record.

| RPC method | Parameters | Result |
|---|---|---|
| `run.restore` | `{run_id}` | `RestoreOperation` |
| `run.restore_status` | `{run_id}` | Ordered array of attempts |
| `node.restore_revision` | `{workflow_id, node_id, commit_id}` | `RestoreOperation` with null `run_id` |
| `node.restores` | `{workflow_id, node_id}` | All node attempts, ordered by operation ID |
| `run.recover_restore` | `{operation_id, confirmation?}` | Resolved `RestoreOperation` |

Recovery requires `confirmation: "processes_stopped"`; omitted/null is a refusal.
Unknown fields and wrong types are invalid parameters, not force options.
Restore/recovery have 120-second client deadlines and a 110-second core budget.
Requests remain serialized; long restoration requests may delay other controls.

New databases create `restore_operations` with a nullable `run_id` foreign key
and a unique active-operation index. Databases from the run-only implementation
are upgraded in one immediate transaction: copy into the nullable schema,
preserve every row/JSON record, ID, active flag, session barrier and AUTOINCREMENT
watermark, replace the table, and recreate the unique active index. Failure
rolls back the upgrade; it never clears an interrupted restore.

Existing JSON records with numeric `run_id` remain readable. New direct commit
selections use null. The Rust `RestoreOperation.run_id` field is now `Option<i64>`;
run-selected responses still serialize the same numeric value. No run, graph or
metadata-history record format changes. Reservation, real journal ID and session
barrier still commit together.

Protocol version 1 is retained for the additive methods; restart an older backend
after rebuilding and do not downgrade the backend against an upgraded database.
This is a targeted journal upgrade, not a general backup/downgrade facility.

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
- Manual exact-commit selection and restoration of preserved checkpoints, even
  without any agent runs; no invented run provenance.
- Invalid/prefix/revset/missing/foreign commit refusal; direct selections still
  require fresh sessions and obey shared downstream/run guards.
- Journal schema upgrade with real legacy serialized operations, unresolved
  reservations, retained ID allocation, idempotent reopening, and rollback when
  late index creation fails.

Native [revision browsing and exact previews](22-native-revision-history.md) are
available separately. Downstream rebasing, cross-change transplantation,
historical prompt/conversation replay and indefinite native-object retention
remain outside this restoration slice.

### Manual exact-commit CLI smoke

The built CLI passed a disposable, agent-free check: create a manual revision A,
leave newer edits B unsaved, restore A by full commit ID, restart the backend,
then restore the first operation's preserved checkpoint. The files returned to B,
both operations retained the same node/change, and `run list` remained empty.
The disposable backend was stopped and socket removal confirmed. Evidence is
under ignored `target/exact-restore-acceptance.*/acceptance.json`; its editing
workspace was turn-local scratch, not a persistent folder to reuse.

The retained real-Codex fixture's prior run-selected restore record also remained
identical when read through the upgraded backend. No new real-agent run was
performed for the exact-commit command; fresh-session and uncertain-operation
behavior have separate native-jj/fake-ACP coverage.

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
