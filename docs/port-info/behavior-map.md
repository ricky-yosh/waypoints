# Frontend behavior map

Starting inventory, not a claim of exhaustive parity or a completed source audit.
Bootstrap reconciliation checked app/workspace entry points, the documented
desktop workflows, and existing workspace test scenarios (including sidebar
focus, stale selection, draft preservation, graph scaling, and workbench state).
No additional baseline UI features were adopted. Reinspect each capability's
implementation and tests when its port begins.
Read [current desktop behavior](../05-ui-and-diffs.md) and current implementation
before porting each row. Record coverage and intentional differences here.

Paths below are relative to the repository root; links point to the reference
implementation. Rune implementation/coverage is pending for every row.

| Capability | Reference | Behavior to preserve |
|---|---|---|
| Startup and native shell | [`lib.rs`](../../crates/waypoints-app/src/lib.rs), [`main.rs`](../../crates/waypoints-app/src/main.rs) | Initialization, menus, shortcuts, window geometry, shutdown, preference-load errors |
| Projects and workflow selection | [`workspace.rs`](../../crates/waypoints-app/src/workspace.rs) | Add/remove sidebar entries without deleting projects, per-project refresh/retry, expanded state, selection, unavailable projects, responsive layout |
| Graph and workbench entry | [`workflow_graph.rs`](../../crates/waypoints-app/src/workflow_graph.rs) | Dependency direction, node statuses including Unknown, selection, keyboard opening, previews, retained camera context |
| Assignment and workbench state | [`node_workspace.rs`](../../crates/waypoints-app/src/node_workspace.rs) | Per-node draft/tab context, save/preview, refresh reconciliation, unsaved warning, close/focus return, expand/restore |
| Runs and output | [`node_runs.rs`](../../crates/waypoints-app/src/node_runs.rs) | Fresh/continued run eligibility, separate follow-up, structured Markdown/reasoning/tools, cancellation, stable tool updates, permission replies/expiry, replay and truncation notices, follow-tail/jump-to-latest |
| Exact patch review | [`node_workspace.rs`](../../crates/waypoints-app/src/node_workspace.rs) | Explicit current/run/revision target, exact commit, file statuses, rename/binary/conflict/empty/error cases, selection/copy, syntax fallback, split/unified, wrap, per-file position, current-target stale notice |
| History | [`node_history.rs`](../../crates/waypoints-app/src/node_history.rs) | Run versus native history, snapshot outcome, session references, pinned pagination cursor, retain loaded pages on older-page failure, review handoff, freshness notice |
| Preferences and appearance | [`preferences.rs`](../../crates/waypoints-app/src/preferences.rs), [`persistence.rs`](../../crates/waypoints-app/src/persistence.rs), [`settings.rs`](../../crates/waypoints-app/src/settings.rs) | Theme/interface zoom, sidebar and window state, safe handling of invalid/newer files; use a separate Rune file during development |
| Fixture and native checks | [`screenshots.rs`](../../crates/waypoints-app/src/screenshots.rs), [`workspace_fixtures.rs`](../../crates/waypoints-app/src/workspace_fixtures.rs), [`workspace_tests.rs`](../../crates/waypoints-app/src/workspace_tests.rs), [`workspace_qa.rs`](../../crates/waypoints-app/src/workspace_qa.rs) | Reuse scenarios and assertions about outcomes, not GPUI harness APIs |
| Backend boundary | [`client.rs`](../../crates/waypoints-backend/src/client.rs), [`protocol.rs`](../../crates/waypoints-backend/src/protocol.rs), [`activity.rs`](../../crates/waypoints-backend/src/activity.rs) | Reuse actual client/data contracts, structured errors and bounded transcript behavior; no direct database/jj access |

## Preserve distinctions, not accidental limitations

- Run completion and snapshot success are separate facts.
- Graph dependencies, native changes, code conflicts, and session eligibility
  must not be inferred from visual position or conflated.
- Historical review remains pinned; do not silently replace it with current code.
- Assignment refresh must not discard an unsaved draft.
- A stale async response must not overwrite the newly selected node/project.
- Closing a workbench does not cancel its run.
- UI errors must preserve actionable backend diagnostics rather than parse
  English messages into invented failure categories.

Known behavior is documented honestly: full History refresh currently replaces
the view with loading/error, while a failed older-page request retains prior
rows. Improving that is possible, but record the intentional change.

## New canvas behavior

Continuous pan/zoom and presentation-only node dragging are migration goals,
not claims about existing GPUI behavior. Define and test camera anchoring,
selection, screen-space overlays, and input/focus interaction in milestone 2.
Do not silently introduce graph editing or persisted position formats.

## Outside baseline parity

The current desktop documentation says workflow/native-change/workspace
provisioning still uses the CLI and history browsing does not restore code.
Do not turn backend restore/rebase/delete/provisioning APIs into new UI features
as part of a mechanical port.

Backend safeguards still apply to every exposed operation. Consult
[frontend-facing backend contracts](../25-backend-frontend-readiness.md) and the
[current status map](../20-core-backend-status.md) when checking scope.

## Coverage recording

As each capability lands, add its Rune source/test paths and native evidence.
Do not mark parity complete from screenshots alone. Resolve discrepancies
between this inventory, current code, and docs before implementing assumptions.
