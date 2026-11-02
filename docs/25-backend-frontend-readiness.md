# Frontend-facing backend support

These APIs are shared by the CLI and future GUI. The GUI must use
`waypoints_backend::Client`, not open the database or invoke jj independently.
They do not introduce a second workflow engine, automatic repair, or inferred
agent-context tracking.

## Repository and base discovery

```sh
waypoints repository inspect --project . --json
```

RPC `repository.inspect {project_path}` and client `inspect_repository(project)`
return canonical `project_path`, canonical `repository_path`, `native_operation`,
and sorted local `bookmarks`. The client resolves relative paths; raw RPC requires
an absolute path. `repository_path` follows the existing workflow-base convention:
the jj workspace root used for native access, not a portable repository UUID.

Each bookmark reports its literal name, ref `conflicted` flag, and complete
`added_targets` / `removed_targets`. Targets contain an exact `revision` and a
native `divergent` flag. Ref conflicts, change divergence, and code conflicts are
separate facts. Remote-only and deleted bookmarks are omitted. No base is guessed.
Actual base selection resolves again using the existing workflow-base checks.

Discovery uses readonly queries pinned to one native operation. It does not
snapshot dirty files, import Git, merge concurrent operation heads, select a base,
or create any workflow records. Missing/non-jj folders and concurrent operation
heads are errors. The total budget is four seconds, with existing two-second,
1 MiB per-stream native bounds and a 900 KiB serialized result limit.

## Explicit stale-checkout update

```sh
waypoints node workspace update-stale --workflow 1 --node 2 --json
```

RPC `node.workspace.update_stale {workflow_id, node_id}` and client/core
`update_stale_node_workspace` return `{outcome, workspace}`. Outcome is `updated`
or `already_fresh`; the workspace uses the existing `NodeWorkspace` shape.
A fresh checkout is a readonly no-op, including when files are dirty.

This operation requires a ready saved dedicated allocation, matching native
repository/change/workspace identity, exclusive ownership, and mutable unique
native state. Active/interrupted runs, unfinished change creation, incomplete
allocations, and unresolved dependency/restore operations are refused. It does
not recreate directories, switch changes, forget workspaces, resolve conflicts,
clear session barriers, or rerun agents.

Before mutation the allocation is durably marked incomplete. Native
`jj --at-op=@ --quiet --config snapshot.auto-update-stale=false workspace update-stale`
runs only after preflight. Readiness is restored only through the same physical
checkout verification used by `node workspace verify`. Preflight has a 40-second
budget; mutation/postflight has 70 seconds, including the 60-second native mutation
limit; client deadline is 120 seconds.

Native errors, diagnostics, timeouts, or failed postflight/readiness writes can
follow file changes. The allocation stays incomplete across restart and refuses
replay. Stop writers, inspect native history/files, reconcile manually if needed,
then explicitly verify. Verification remains readonly with respect to jj/files.

In particular, jj 0.46 can snapshot unsaved stale edits on their old operation,
creating divergent operation heads. The explicit `@` operation policy refuses to
merge those heads automatically. Tested dirty files remain on disk, and Waypoints
reports failure/incomplete rather than claiming successful reconciliation. Native
history may have changed even when the command fails. This is not a general
dirty-checkout conflict resolver.

## Structured errors

JSON-RPC errors preserve their original `code`, diagnostic `message`, and optional
`data`. `BackendError::rpc_error()` returns a remote `RpcError`, or `None` for
local startup/transport/protocol failures. `RpcError` exposes `code()`, `message()`,
`data()`, `failure()`, and `is_busy()`.

Expected storage refusals use `-32000` and typed data tagged by `kind`:

| Kind | Fields |
|---|---|
| `missing_workflow` | `workflow_id` |
| `missing_node` | `workflow_id`, `node_id` |
| `missing_run` | `run_id` |
| `node_busy` | `node_id`, `run_id` |
| `workspace_not_ready` | `node_id` |

These are explicit causes at the core lookup/reservation/readiness checks, not
English-message classification. This is a bounded initial set, not exhaustive
classification of every safety refusal. Other errors remain diagnostic failures;
clients must keep a generic error path. Unknown future data and legacy errors
without data remain readable. Standard invalid request/method/parameter errors
keep their JSON-RPC codes.

Scheduler refusal is distinct: `-32001` with `data: {"kind":"backend_busy"}` means
the request was **not started**. `RpcError::is_busy()` recognizes that exact
contract, not an arbitrary message mentioning “busy”.

## Scheduling and run controls

One storage lane serializes ordinary requests, metadata writes, and run snapshot
preparation. The server continues to frame connections, process live worker
updates, and service this explicit bypass list while that lane is occupied:

- `backend.info` and `backend.shutdown`;
- `run.status` for retained live runs;
- `run.events`, `run.cancel`, and `run.permission`.

Historical status lookups and other ordinary requests do not bypass storage.
They receive the busy refusal if the lane is occupied. There is no server queue
of mutations waiting to start after their clients have timed out. The Rust client
retries **only** this explicit not-started refusal, at 50 ms intervals within the
original request deadline. It never retries transport errors, lost responses,
or uncertain domain failures.

Framing/response work is bounded to 64 concurrent tasks, 1 MiB per frame, with
five-second reads/writes. Saturated admission can close a connection; this is not
a guarantee under unbounded load. Slow individual socket clients no longer own
the storage dispatch lane.

Cancellation/permission replies affect the owned live worker immediately; run
metadata writes may wait for storage. Live cancellation is an acknowledgement
of a request, not proof of completed cleanup or durable persistence. Saved
reservations remain conservative during that delay. Successful run completion
and its snapshot result are not exposed until the metadata write succeeds.
After a crash, only persisted state is available and unfinished reservations
still require explicit recovery.

Shutdown acknowledges promptly, then waits for the owned storage/native outcome
and agent/snapshot cleanup before releasing ownership locks. No native mutation
is cancelled by dropping its future. External jj/filesystem writers remain
outside Waypoints' serialization guarantee.

## Evidence

Disposable native-repository and CLI/backend tests cover canonical readonly
discovery, literal names, empty repositories, ref/code conflicts and divergence;
clean/fresh workspace updates, preserved dirty failure, unsafe allocation refusal,
restart and injected readiness-write failure; typed errors and unknown-data
fallback; and live cancellation/permission/status/events during gated native work.
The scheduling tests also cover conflicting mutation refusal and orderly shutdown.
Existing crash/finalization, permission-expiry, startup and concurrent-client
regressions remain part of the full headless suite. These checks use deterministic
test agents, not new real-Codex acceptance.
