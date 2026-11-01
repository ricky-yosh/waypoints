# Readonly restore impact

Inspect what an upstream restore could affect before deciding whether to change
code. This is a **conservative impact report**, not a speculative rebase,
conflict simulation, apply token, or permission to restore downstream work by itself.
[Checked application](24-downstream-restoration.md) requires a fresh equal report
and separate explicit confirmation.

```sh
waypoints node restore-impact --workflow 1 --node 2 --commit FULL_COMMIT_ID --json
```

Use an exact, available, 40-character lowercase native commit ID from this
node's own change. A saved run's snapshot commit, manual revision, or preserved
checkpoint works. Prefixes, change IDs, bookmarks, revsets and another change's
commit are refused. The current source must have one visible revision.
The source need not have an allocated workspace merely to inspect impact.

Without `--json`, the CLI prints the same report as pretty JSON. It can
auto-start the backend. Rebuild and restart an older backend to use the additive
protocol-1 method:

| Surface | Contract |
|---|---|
| Core / backend client | `node_restore_impact(workflow_id, node_id, commit_id)` |
| JSON-RPC | `node.restore_impact {workflow_id, node_id, commit_id}` |
| Response | `RestoreImpact` |
| Bounds | 120-second client deadline, 110-second core budget; native queries retain their two-second/1 MiB-per-stream limits; serialized report at most 900 KiB |

Unknown fields, including `apply` and `force`, are invalid parameters. An
oversized or incomplete native query fails, rather than returning a truncated
impact list.

## What the report means

For example, restoring `Build API` in this graph can affect both materialized
code and a task that has not created any code:

```text
Build API                source change
  └─ Build login page    native descendant, workspace, session
       └─ Test login     prompt-only downstream task
```

| Field | Meaning |
|---|---|
| `workflow_id`, `node_id`, `repository_path` | Selected source and repository |
| `native_operation` | Exact operation used for native ancestry, workspace registry, mutability and recorded-tree comparison |
| `current`, `selected` | Unique current source revision and exact selected revision |
| `recorded_tree_changed` | Whether those two **recorded** trees differ; false does not prove a restore would be a no-op when files have unsnapshotted edits |
| `parents_match` | Whether their exact native code-parent IDs match; false remains inspectable but current restore cannot apply it |
| `changes` | Source plus all visible native descendants, with exact code-parent IDs, existing conflict flags, immutable flags, and owning saved node IDs |
| `tasks` | Source, native-owned tasks across workflows sharing this repository, and transitive saved-graph descendants of all those tasks |
| `workspaces` | Native workspaces whose recorded commit is in `changes`, including unmanaged workspaces; physical metadata is `current_recorded`, `stale`, or explicitly `unknown` |
| `sessions` | Historical runs grouped by task/session ID; null means no session was established or persisted. `source_node` distinguishes the source's existing fresh-session-on-apply rule from potential downstream context staleness |
| `active_run_ids` | All database-wide active/interrupted run reservations, including unrelated work |
| `observed_blockers` | Known parent/workspace/immutability/downstream/run/reservation obstacles; not an exhaustive apply preflight |
| `notes` | Limits that must remain visible when presenting or interpreting the report |
| `graph` | Saved source-workflow graph, including edges, compared on checked application |
| `restore_watermark` | Last database-wide restore ID; invalidates stale confirmation even after a native no-op |

Each task retains its workflow ID. `native_affected: false` distinguishes a
graph-only relationship from proven native ancestry; it does not claim that the
task's code will be rebased. Its optional `allocation` reports the saved
workspace reference and initial allocation's `ready` flag, **not** checkout
freshness. Missing/forgotten allocations remain visible there even if absent
from the native workspace registry.

Merge descendants retain every native parent ID, including inputs outside the
source subtree. Those unrelated inputs are not themselves descendants.
Native changes without saved owners have empty `node_ids`; they are not hidden.
Cross-workflow native owners and their graph-only downstream tasks are reported,
not silently adopted into the selected workflow.

## Explicit limits and safety

- No snapshot, restore, rebase, checkout update, repair, journal reservation,
  session barrier, prompt edit or agent execution occurs. No new persistence
  schema or saved preview archive is introduced.
- Native queries ignore working-copy edits and pin one operation. If the live
  native operation changes before return, refresh rather than using a mixed
  report. This does not lock outside writers or make filesystem observations
  atomic.
- Physical checkout inspection reads metadata only. `current_recorded` does
  not mean clean files. Unavailable directories/unsupported metadata are explicit
  unknowns, not guesses or automatic repair.
- Unsnapshotted and ignored files are not inspected or backed up. The report
  lists **potential** rewrite targets; it does not calculate resulting commit
  IDs or promise that every listed revision must change.
- Existing `conflicted` flags describe current code. **Future restore/rebase
  conflicts are unknown**, not simulated as clean.
- Session references do not prove causal inputs, actual reasoning freshness,
  harness retention, or continuation eligibility. Applying existing leaf
  restore requires a fresh session on the source; checked downstream application
  bars all pre-existing affected sessions. Broader context policy remains undecided.
  Preview never changes or resumes any session.
- Active runs and unresolved restore/dependency reservations are surfaced when
  the source remains readable; they do not hide the report behind a mutation
  guard. A missing/divergent source, unavailable repository, foreign selector,
  query failure or exceeded bound still returns an error.
- [Ordinary restore](21-saved-result-restoration.md) still refuses saved/native
  descendants. [Explicit checked application](24-downstream-restoration.md) now
  supports a native-aligned same-workflow subtree. Cross-workflow/unmanaged
  reconciliation, automatic conflict/workspace repair and general context policy
  remain separate work.

## Validation

Disposable native-jj tests cover dirty-file readonly inspection and restart,
source sessions, transitive graph-only tasks, native merge ancestry, unmanaged
work, cross-workflow ownership, stale/missing/shared workspaces, existing merge
conflicts, active runs, parent drift/immutable policy, missing/divergent sources,
allocation-free inspection, invalid selectors, and native/serialized output
bounds. CLI/backend/fake-ACP tests cover strict RPC, pretty/compact JSON,
restart, unchanged sessions/graph/journal/files, readable unresolved restoration,
and rejecting a concurrent native rewrite without introducing a preview operation.

These preview checks do not constitute a real-agent downstream restore or a
visual acceptance test; application has its own [validation contract](24-downstream-restoration.md#validation-and-remaining-limits).

### Manual CLI acceptance

A disposable fixture at `target/impact-acceptance.hdncb4gc/acceptance.json`
records a passing built-CLI smoke test: `Build API → Build login page → Test login`.
The first two tasks have native changes and dedicated workspaces; the last is
prompt-only. API A was selected after recording API B and leaving API C unsaved.

The report identified two native changes, three saved tasks and two current
recorded workspaces, and returned the downstream-application blocker. It was
identical after stopping/restarting the backend. Native operation/commit IDs,
graph, files and empty restore journal stayed unchanged; no agent run was
created. The fixture backend was stopped afterwards.

Editing workspace paths in this evidence were turn-local scratch directories,
not durable/reusable workspaces. No GUI or fresh real-Codex run was exercised.
