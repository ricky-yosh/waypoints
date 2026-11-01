# Backend-owned ACP runs and explicit continuation

## Implemented scope

Waypoints can start a fresh ACP session for a saved node's verified dedicated
workspace or explicitly continue a saved session with a follow-up prompt.
Short backend requests control the background agent task; the backend remains
the sole database owner.

The real CLI/backend fresh-run path was manually validated with Codex adapter
2.5.0: Codex created `hello.txt` in the dedicated workspace and completed the run.
Native `jj status`/`diff --git` and `node refresh` confirmed the edit on the same
change with a new commit ID. This is separate from the earlier temporary-client
[Codex spike](12-codex-acp-spike.md).

Continuation is implemented and validated with a deterministic ACP peer through
the real CLI/backend, including restart and replay. The subsequent manual
real-Codex continuation check was reported successful by the user.

Successful turns automatically snapshot their workspace and record the exact
native revision before releasing ownership. This is fake-peer/native-jj tested
and passed the manual real-Codex check: run 4 saved commit
`7ea584b9fb4be5b03b1b7b510b7861a9da93dc0d` on the original change, matching
read-only `node refresh` without a client-side `jj status`.

Explicit interrupted-run inspection and recovery are now implemented, tested
with backend crashes, a fake ACP peer, and native jj. Recovery requires caller
attestation that old writers stopped; it is not automatic process detection.
There is no automatic retry, workflow-edge execution, transcript archive,
installer, or GUI integration.

## Informational upstream hint

New runs and continuations best-effort record the direct dependencies of a rooted
workflow as `run.upstream_inputs`: a list of `{parent, commit_id}`. `parent` is
the saved workflow base or an upstream node. The base stays pinned to its saved
commit; node inputs use their unique current recorded revisions. Each continuation
captures new observations without replacing the previous run's observations.

```sh
waypoints run status --run 1
waypoints run inputs --run 1 --json
```

Human-readable `run status` appends a small hint. `run inputs` / RPC `run.inputs`
returns `{status: "unchanged"}`, `{status: "changed"}`, or
`{status: "unknown", detail}`. The existing JSON `run status` shape is unchanged:
use `run inputs` for the live comparison; `run.upstream_inputs` holds only the
saved observation. Both durable queries can auto-start the backend.

Comparison uses exact commit IDs and dependency identities. Upstream code
rewrites, native description-only rewrites, and dependency changes count;
prompt-only edits, downstream-only edits, and unsnapshotted upstream files do not.
An unavailable/missing/divergent upstream, deleted target node, or failed query
reports unknown rather than claiming unchanged. Older runs, legacy unbased
workflows, and runs whose optional input capture failed also report unknown.

Capture adds at most 500 ms of optional bookkeeping before reservation/launch;
failure or timeout does not block a run. Comparison is read-only with a
four-second total budget and the existing bounded native queries. Neither
operation snapshots code, changes metadata history, injects text into agent
prompts, requires acknowledgement, blocks execution, or reruns anything.

This is a direct graph-revision hint, **not full context-staleness tracking or
proof of what the agent actually read**. It does not check graph/native ancestry
alignment, physical checkout freshness, transitive inputs, or agent reasoning.
Observations are not atomic with external jj writers or agent execution; no
native objects are pinned against garbage collection.

## CLI

First allocate and verify a node workspace using the
[existing workspace commands](11-backend-cli.md). Stop an older backend after
rebuilding the executable so it picks up the new methods.

For an installed ACP executable, explicitly select its path:

```sh
cargo run -p waypoints-cli -- run start \
  --workflow 1 --node 2 --executable "$(command -v codex-acp)" --json
```

Only run an executable you trust.
Waypoints launches the selected file directly without shell parsing or arguments;
relative executable paths resolve from the CLI's current directory.

The response contains `{run, permission}`. Save `run.id`; starting returns
`starting` before adapter initialization completes. A lost start response is an
uncertain outcome: inspect `run list`, never automatically repeat the start.

```sh
cargo run -p waypoints-cli -- run list --workflow 1 --node 2 --json
cargo run -p waypoints-cli -- run status --run 1 --json
cargo run -p waypoints-cli -- run events --run 1 --after 0 --json
```

`run events` polls once, returning `{status, events, next, gap}`. Pass `next` as
`--after` on the next poll. If `gap` is true, some requested output expired:
surface the gap rather than presenting an incomplete stream as complete.
This checkpoint does not include a continuous terminal renderer.

When `status.permission` is present, inspect its `tool_call` and offered
single-use `options`. Reply using that permission ID and an exact option ID:

```sh
cargo run -p waypoints-cli -- run permission \
  --run 1 --permission 1 --option OPTION_ID_FROM_STATUS --json
cargo run -p waypoints-cli -- run cancel --run 1 --json
```

There is no implicit approval, including in noninteractive use. Blanket
`allow_always` options are not exposed. Stale permission IDs, unknown options,
and duplicate replies are rejected. An unanswered request expires after
60 seconds and cancels the run without granting permission.

Start, continue, list, status, and recovery inspection can auto-start the backend.
Events, cancellation, permission replies, and recovery confirmation require an
existing backend, so they do not conceal a restart.
Without `--json`, these commands currently print pretty JSON.

## Explicit continuation

After a run stops with confirmed process cleanup, select its ID and send a new
prompt:

```sh
cargo run -p waypoints-cli -- run continue --run 1 \
  --prompt 'Extend hello.txt with a second greeting. Work only in this workspace.' --json
```

This returns a **new run ID**, with `continued_from` referencing the selected run.
Use that new ID for status, events, approvals, cancellation, and the next
continuation. The follow-up is saved on the new run; the node's original prompt
and the earlier run are unchanged. `run start` still explicitly creates a fresh
session using the saved node assignment.

Rules:

- The selected run must be `completed`, `cancelled`, `failed`, or `recovered`, with a saved
  session ID. Starting, active, cancelling, finalizing, and interrupted runs are ineligible.
  An adapter that failed before creating a session cannot be continued.
- Select the latest run for that session. Continuing an older run is not a
  conversation fork or rollback; Waypoints refuses it and names the newer run.
  The harness may also have changed its session history outside Waypoints.
- [Restoring a saved result](21-saved-result-restoration.md) bars continuation of
  every pre-existing session on that node. Explicitly start a fresh session;
  selecting old code never silently rewinds agent memory.
- The same executable path, workspace directory, and jj change are reused.
  Read-only checkout verification runs again; dirty local edits are allowed.
  A changed node association or externally switched workspace is refused rather
  than retargeting the session. Executable paths are saved, not binary versions.
- The new adapter must advertise `agentCapabilities.loadSession: true`.
  Waypoints sends `session/load` with the saved ID, exact workspace, and no MCP
  servers. It waits for the load response, then explicitly selects
  `workspace-write` again **before** sending the follow-up. ACP permits an empty
  load result without repeating the session ID or modes.
- Every event carries `origin: "replay"` or `"live"`. Updates received before the
  load response are replay, including historical tool calls. Updates outside
  that loading interval are live. Replay never grants permission or applies an
  edit; clients must not render it as newly executed work.
- Replay uses the same bounded buffer and cursor as live output. Large history
  may produce `gap: true`; this is not a complete conversation-history API.
  Setup, including loading/replay, still has the 30-second deadline.
- Unsupported loading, unavailable history, mismatched session IDs, mode errors,
  and malformed responses fail the new run. There is **no session/new fallback**
  and no follow-up is sent before loading/mode selection succeed.
- The new run reserves the node and workspace before launching. Existing
  approval, cancellation, shutdown, and crash rules apply, including cancellation
  during load. An uncertain continue response must not be automatically retried:
  inspect `run list` first. A failed load retains its selected session reference;
  after fixing the cause, explicitly retry from that latest failed run.

## Lifecycle and ownership

- A run captures the prompt, workspace path, jj change ID, and selected executable.
  Later node-prompt edits do not alter the active assignment.
- The node workspace must pass the existing read-only checkout verification.
  Dirty local edits are allowed; files are not cleaned or reset.
- Durable reservations prevent overlapping starts for the same node or directory.
  Reassociation is refused while the node is reserved.
- Up to four runs can be active. Each owns its adapter process and ACP connection.
  The backend serializes metadata writes; workers send lifecycle/event messages
  over bounded channels and never open the database.
- The handshake negotiates ACP version 1, creates or loads a session, explicitly
  selects `workspace-write` mode, and sends the prompt. An incompatible agent
  fails rather than silently using a different permission mode.
- No client filesystem, terminal, or authentication callbacks are advertised.
  The agent must already be authenticated and supply its own coding tools.
  This is not a Waypoints OS sandbox: permission enforcement depends on the
  trusted agent. Unsupported callbacks receive an error.
- After `end_turn` and confirmed cleanup, the run enters `finalizing`, then
  `completed` with a separate snapshot outcome. Explicit agent cancellation produces
  `cancelled`. Other stop reasons and protocol/process failures produce `failed`.
  These are execution outcomes, not code-review approval or jj snapshot success.
- Adapter setup has a 30-second deadline; a run has a one-hour deadline.
- Cancelling returns promptly with `cancelling`. The worker sends ACP cancel and
  allows up to 500 ms for a response, then stops the owned process group and reaps
  the adapter. Cleanup also runs after normal completion or errors. The workspace
  is released only after confirmed cleanup and, for successful turns, recording
  the snapshot outcome.
- Closing the CLI does not cancel the run. Another client can inspect/control it.
- Orderly backend shutdown requests cancellation and waits for cleanup while
  retaining database/socket ownership locks. The shutdown response acknowledges
  the request, not completed process cleanup.
  If finalization has begun, shutdown waits for its bounded snapshot operation;
  it does not cancel it. `run cancel` likewise leaves `finalizing` unchanged.

States are `starting`, `running`, `awaiting_permission`, `cancelling`,
`finalizing`, `completed`, `cancelled`, `failed`, `interrupted`, and `recovered`.

## Automatic revision recording

Finalization belongs to the headless backend, not the CLI or future GUI:

```text
agent end_turn → confirmed process cleanup → finalizing
→ verify workspace → native jj snapshot → save outcome and release reservation
```

No explicit CLI snapshot call is required. Closing the originating client does
not stop finalization. Status/list/event polling never triggers a snapshot.

`run.snapshot` is separate from `run.state`:

- `{"status":"saved","revision":{...}}`: the exact change ID, commit ID,
  description, and conflict flag returned by the native snapshot command.
- `{"status":"failed","detail":"..."}`: the agent completed, but an exact
  snapshot was not confirmed/recorded. `run.state.status` is still `completed`,
  not an agent failure. Inspect the detail and native state; no agent or snapshot
  retry occurs automatically.
- `{"status":"not_recorded"}`: active/finalizing, cancelled, failed, or a legacy
  run from before this feature. Old completed runs are not retroactively saved.

The backend durably saves `finalizing` before preparing native work. It reuses
read-only node/workspace/checkout verification, then runs the snapshot in a
background task with a 60-second total deadline. Other clients can inspect status
while native snapshotting runs. Verification itself remains a serialized,
bounded native query, like existing request preflight.

The native command is `jj --at-op=@ --config snapshot.auto-update-stale=false
--quiet log --no-graph -r @ -T <revision-json-template>`. It snapshots and reports
the exact resulting revision in one command, rather than querying a moving head
afterwards. A stale, switched, missing, divergent, or immutable checkout is
refused; finalization never repairs it, resets files, or creates a child change.
Normal jj snapshot/rebase behavior applies to the existing change.

Native ignore/tracking rules apply: this records the current workspace, including
pre-existing/manual edits that jj tracks, not exclusively edits attributed to
the agent. Diagnostics such as an oversized new file are surfaced as snapshot
failure rather than silently claiming all output was recorded. Even an error
may follow a native mutation; there is no rollback. A no-change turn can retain
the existing commit ID. Saving a revision is not review approval, and storing a
commit ID does not override native jj retention/garbage-collection rules.

The final outcome and reservation release are written together in one database
update. A later turn or manual edit cannot change an earlier run's saved revision.
Cancelled/failed agent turns preserve their files without automatic snapshotting.
There is no finalization-retry command. Interrupted reservation recovery below
does not retry or reconstruct a lost finalization.

Waypoints reservations serialize its own runs, not external editors or native jj
commands. Do not concurrently rewrite the workspace during finalization.

## Persistence, bounds, and crash behavior

An additive `agent_runs` table stores captured prompts, executable paths,
session references, continuation links, diagnostic adapter process/group IDs,
snapshot revisions/failures, and outcomes.
It does not store text streams or conversation history.
Older records remain readable; missing snapshot metadata means `not_recorded`.

Live event buffers retain at most 128 events or 256 KiB per run, whichever limit
is reached first. The backend retains at most 32 live run records, evicting old
finished buffers as new runs start. Missing buffers after eviction/restart return
an explicit error; durable status remains available. Captured prompts are limited
to 128 KiB and incoming ACP frames to 64 KiB. Oversized frames fail the run rather
than being silently truncated. Adapter stderr is discarded, not accumulated.

On restart, unfinished saved runs become `interrupted` and **retain their
reservations**. A backend crash cannot prove external processes stopped. Saved
PIDs are diagnostic only: they may have been reused and must never be signalled
automatically after restart.
For a run interrupted during finalization, cleanup had already completed, but
the native snapshot outcome may not have been durably recorded. Its interruption
detail identifies this uncertainty. The backend never reconstructs a run revision
from whatever happens to be the current head or automatically repeats the snapshot.

The first interruption's diagnostic and structured `phase` survive further
restarts: `execution` (including unconfirmed process cleanup), `finalization`,
or `unknown` for legacy records without phase metadata. No phase is guessed
from old free-text diagnostics.

Do not delete files, replay prompts, or silently assume a workspace is safe.
Processes that detach from the adapter's process group are outside the cleanup
guarantee and must also be considered during manual process inspection.

Ordinary requests remain serialized. A different slow native operation or slow
socket client can still delay control requests. This checkpoint removes the
agent's entire execution time from the request handler; it does not promise
general concurrency for all native/storage operations.

## Recovering interrupted runs

First inspect the interrupted run and current workspace:

```sh
cargo run -p waypoints-cli -- run recovery --run 1 --json
```

This returns `{run, workspace}`. The saved run includes its original interruption
diagnostic/phase, assignment, session reference (if any), workspace, change, and
diagnostic process ID. `workspace.status` is either:

- `verified`, with `current_revision`: a current read-only observation, **not**
  the interrupted run's output or saved snapshot.
- `blocked`, with `detail`: failed allocation, checkout, native, or storage checks.

Inspection does not release the reservation or modify files, history, working-copy
metadata, readiness flags, or the run. A verified workspace does **not** prove
that old processes stopped.

Before confirming recovery, positively identify and stop all old writers:
the adapter, agent tools/descendants, and any native jj finalization process.
Inspect their actual commands, working directories, and ancestry using the OS's
process tools. A saved PID can have been reused; do not blindly signal that PID
or infer safety merely because it is absent. A descendant may outlive its adapter.
For `finalization` or `unknown`, include native snapshot writers in the check.
Waypoints never sends a signal as part of recovery.

Only after that inspection and cleanup, explicitly attest:

```sh
cargo run -p waypoints-cli -- run recover --run 1 \
  --confirm-processes-stopped --json
```

The flag is required, including in noninteractive use. At the RPC boundary,
`run.recover` requires `confirmation: "processes_stopped"`; missing, false,
unknown, or extra parameters are rejected. This is a caller attestation, not
independent OS proof or an automatic cleanup feature. Do not set it blindly.

The backend refuses any run with a still-owned live task. Only an interrupted
run retaining its reservation is eligible. It freshly verifies the already-ready
allocation, current node/change binding, native workspace registry/root, and
recorded checkout, even if an earlier inspection passed. Dirty edits are allowed.
Missing directories, external switches, stale checkouts, incomplete allocations,
missing/divergent changes, and unavailable jj block release. No force, checkout,
repair, reset, or directory replacement is performed.

On success the backend atomically saves `state.status: "recovered"` and releases
that run's reservation, conditional on its saved record remaining unchanged.
The original interruption diagnostic and phase remain on the recovered state.
Session, prompt, diagnostic PID, and snapshot metadata are retained unchanged.
`recovered` means reservation reconciliation, **not** agent success, cancellation,
review approval, or proof that the interrupted turn's work was saved.

Repeated recovery returns that same record without releasing a newer run's
reservation or re-verifying a workspace now in use by another turn. Failed
recovery leaves the reservation and interruption untouched. Recovery requires
an existing backend; inspection can start one and mark unfinished saved runs
interrupted first.

No agent or snapshot runs automatically. After recovery, explicitly choose:

- `run continue --run 1 --prompt '...'` to load the saved session, if it exists.
  The normal latest-run/capability/workspace checks still apply. Inspect partial
  work instead of blindly repeating the previous prompt.
- `run start ...` for a fresh session using the node assignment in the existing
  workspace, with partial edits preserved.

An interrupted finalization's current head is never assigned to the old run.
A later successful turn may record its own exact revision of the current
workspace, but it cannot retroactively fill the interrupted run's snapshot.

## Diffs and jj

Live output preserves ACP update objects, including proposed diffs. A diff event
does not prove the edit completed: approvals can be denied. Polling output never
applies a diff or grants a permission.

Structured edit diffs are not guaranteed: the real Codex run wrote `hello.txt`
through `exec_command` (a shell heredoc), reporting an execute tool and terminal
output but no structured ACP edit diff. Native jj remains authoritative for the
actual file changes.

Successful run completion includes the automatic recording boundary above.
Ordinary `node refresh`, workspace verification, and polling remain read-only;
they never snapshot additional edits.

## Validation

Integration tests launch the real executable with temporary databases and native
jj workspaces. The fake ACP peer uses Python 3 and never calls a model.

Covered:

- Status and graph queries while an agent awaits approval.
- Overlap refusal, captured prompts, session references, and explicit modes.
- Permission denial, approve-once, stale/duplicate/blanket-option rejection,
  and the real 60-second unanswered-permission deadline.
- Cursor polling, bounded output with explicit gaps, and restart loss of buffers.
- Cancellation and shutdown stopping a process group, including a child that
  ignores cancellation.
- Malformed JSON, oversized events, process exit, and successful completion.
- Durable outcomes after orderly restart; interrupted reservations after a
  forced backend crash, with no automatic prompt replay.
- Explicit continuation after adapter/backend restart, fake harness-owned
  conversation-only state, same-change edits, unchanged node assignments and
  earlier run records, replay/live markers, and cursor polling.
- Load capability absent/false, load failure, wrong-session response/update, mode
  rejection, invalid follow-up/source, missing session, stale source selection,
  and switched native checkout, without fresh-session fallback.
- Approvals and denial on follow-ups, cancellation during load and execution,
  descendant cleanup, and bounded replay with explicit gaps.
- Automatic native snapshots on fresh/follow-up success, exact revision contents,
  unchanged original working-copy revision, and frozen metadata across later edits
  and backend restarts, without a client-side mutating jj call.
- Reservation retention and status access during a blocked snapshot, cancellation
  semantics, and orderly shutdown waiting for finalization.
- Preflight refusal after an external switch; no snapshots on cancelled/failed
  agent turns; native errors/warnings and malformed output after a real mutation.
- Crash during finalization retaining its reservation without agent/snapshot replay.
- Recovery requiring explicit attestation, preserving dirty files/checkout bytes
  and native operation IDs, while inspection leaves an old descendant running.
- Fresh rechecks refusing missing/switched/stale workspaces after earlier verified
  inspection; live/successful runs and malformed confirmation requests refused.
- Recovered outcomes persisting across restarts, old recovery retries preserving
  newer reservations, explicit session continuation or fresh start without a session.
- Interrupted finalization retaining its phase across restarts and recovery,
  without snapshot reconstruction/retry; legacy phases defaulting to unknown.

### Real Codex automatic-snapshot acceptance check

In a disposable node/workspace:

1. Rebuild and stop an older backend before starting a new turn.
2. Ask Codex to edit a file without running any version-control commands.
3. Poll status until `completed`; require `snapshot.status == "saved"` and inspect
   its exact revision. Do **not** run mutating native commands to make it appear saved.
4. Read the file and use `node refresh` or native `--ignore-working-copy
   --at-op=@` queries to confirm the same change and saved commit.
5. Continue the latest run with another edit; confirm a new recorded revision on
   the same change while the earlier run's revision remains unchanged.

The manual check passed for run 4: status reported `completed` and snapshot
`saved`; its commit ID matched read-only node refresh and the requested line was
present in `hello.txt`. Recovery tests deliberately crash only disposable fake
agents/backends; this change did not crash or recover the user's real Codex session.
Deliberate finalization retry remains a future slice.
