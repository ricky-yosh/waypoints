# Waypoints: plan-driven agent workflows

Status: early headless implementation on macOS/Linux. Core/backend/CLI support
persistent rooted workflows, native jj changes and on-demand workspaces, ACP runs
and continuation, automatic successful-turn snapshots, same-change run/commit restoration
and explicitly confirmed same-workflow downstream rebasing,
metadata undo/redo, native reparenting, multi-parent native creation,
native deletion with retained workspace folders,
read-only native code review, paginated revision history and restore-impact inspection,
an informational upstream-revision hint, and undoable prompt-only move/delete.
Explicit recovery handles clear interrupted outcomes; ambiguous states remain
blocked. The desktop app crate is still a placeholder; GUI/backend integration
is not implemented.

**V1 reflects native jj revisions and conflicts.** Users decide what changed
assumptions require review or rerunning; a separate downstream-context-staleness
badge or execution gate is not required. Physical-checkout diagnostics and
restore-specific fresh-session safeguards remain in place. See the
[scope decision](docs/02-architecture.md#decision-reflect-native-jj-changes-not-inferred-context-staleness).

Start with the [current core/backend status and documentation map](docs/20-core-backend-status.md)
for implemented boundaries, remaining discussion items, and links to detailed contracts.

## Try the CLI

For an existing jj project with a local `main` bookmark, explicitly save its base:

```sh
cargo run -p waypoints-cli -- workflow create --project . --name "First workflow" --base main
cargo run -p waypoints-cli -- workflow list --project . --json
```

Choose another bookmark or `--base-revision` when appropriate. Omitting both
selectors still creates a legacy unbased workflow, not a workflow implicitly
rooted at `main`; see [base compatibility](docs/16-workflow-bases.md#compatibility-and-existing-workflows).

The first workflow command starts the local backend automatically. Later commands
reuse it; the CLI never opens the database directly.

```sh
cargo run -p waypoints-cli -- backend status --json
cargo run -p waypoints-cli -- backend stop
```

Stopping the backend preserves saved workflows. The next workflow command starts
it again. See [backend and CLI usage](docs/11-backend-cli.md) for paths, isolated
test data, the local protocol, and current limitations.

## One line

Plan branching coding-agent work around evolving jj changes, inspect exact
revisions, and explicitly choose what to run or revisit.

## Short description

Product direction, not a list of implemented GUI/backend capabilities:

Waypoints turns a plan into a graph of assignments. A node can own one evolving
jj change with many exact revisions, and its agent work can use fresh sessions or
explicit continuation over ACP. Review diffs, branch parallel work, revisit an
earlier revision, and see jj's resulting changes and conflicts. The headless
implementation is Rust; the planned desktop client uses GPUI Kit. Codex is the
manually validated adapter, not a guarantee that every ACP agent is supported.

## Documentation

| File | What it covers |
|---|---|
| [`20-core-backend-status.md`](docs/20-core-backend-status.md) | Start here: current capability inventory, documentation map, and unprioritized discussion backlog |
| [`01-concept.md`](docs/01-concept.md) | Product direction, terminology, and non-goals |
| [`02-architecture.md`](docs/02-architecture.md) | Ownership and scope decisions; explicitly labeled future sketches |
| [`03-acp-and-agents.md`](docs/03-acp-and-agents.md) | Implemented ACP boundary versus protocol and adapter research |
| [`04-jj-snapshots.md`](docs/04-jj-snapshots.md) | Evolving changes, exact revisions, native conflicts, and restoration |
| [`05-ui-and-diffs.md`](docs/05-ui-and-diffs.md) | Future UI sketch and trustworthy diff presentation |
| [`06-stack-and-storage.md`](docs/06-stack-and-storage.md) | Current stack, storage ownership, and optional prototype approach |
| [`07-spikes-and-build-order.md`](docs/07-spikes-and-build-order.md) | Historical spikes, headless-first direction, and integration acceptance |
| [`08-proa-and-web.md`](docs/08-proa-and-web.md) | Earlier research into an optional web side; not an adopted dependency |
| [`09-ideas-explored.md`](docs/09-ideas-explored.md) | Historical alternatives, not implementation requirements |
| [`10-helpful-tools.md`](docs/10-helpful-tools.md) | Development-tool references and adoption boundaries; not a roadmap |
| [`11-backend-cli.md`](docs/11-backend-cli.md) | Current CLI/RPC reference, transport, paths, lifecycle, limits, and validation commands |
| [`12-codex-acp-spike.md`](docs/12-codex-acp-spike.md) | Historical temporary-client Codex observations and lessons for production |
| [`13-agent-run-lifecycle.md`](docs/13-agent-run-lifecycle.md) | Implemented background-run checkpoint, commands, bounds, cleanup, and remaining work |
| [`14-workflow-history.md`](docs/14-workflow-history.md) | Persistent metadata undo/redo, CLI/RPC, native-history boundary, and validation |
| [`15-native-dependency-editing.md`](docs/15-native-dependency-editing.md) | Rooted code-dependency decision, native abandonment tests, and scope boundaries |
| [`16-workflow-bases.md`](docs/16-workflow-bases.md) | Explicit bookmark/revision starting points, persistence, CLI/RPC, and legacy compatibility |
| [`17-rooted-node-dependencies.md`](docs/17-rooted-node-dependencies.md) | Explicit node attachments, rooted connectivity, native creation checks, and legacy adoption |
| [`18-native-reparenting.md`](docs/18-native-reparenting.md) | Checked native reparenting, code-changing undo boundaries, and unresolved operation safeguards |
| [`19-native-node-deletion.md`](docs/19-native-node-deletion.md) | Native node abandonment, child reconnection, workspace restrictions, and shared recovery |
| [`21-saved-result-restoration.md`](docs/21-saved-result-restoration.md) | Ordinary leaf restoration by saved run or exact same-change commit |
| [`22-native-revision-history.md`](docs/22-native-revision-history.md) | Native evolution browsing, pagination, exact previews, and retention limits |
| [`23-restore-impact.md`](docs/23-restore-impact.md) | Read-only conservative impact preview and confirmation inputs |
| [`24-downstream-restoration.md`](docs/24-downstream-restoration.md) | Confirmed same-workflow downstream restoration, session barriers, recovery, and real-Codex acceptance |

## Decisions so far

| Topic | Decision |
|---|---|
| Agent integration | ACP client, one adapter per agent, no custom agent loop |
| Code history | One evolving jj change per node, many exact revisions; workspaces allocated on demand |
| Native feedback | Reflect jj changes/conflicts; no inferred downstream-context-staleness state for v1 |
| Diff viewing | Headless exact-revision review implemented; inline GUI review and editor handoff pending; no custom multibuffer |
| UI | GPUI Kit (`gpui-kit`); desktop integration pending; see [current stack](docs/06-stack-and-storage.md#gpui) |
| Storage | Backend-owned Turso with plain SQL; SQLite migration and backup procedures are not implemented |
| Prototype | React/TypeScript/React Flow is an optional design experiment, not a required build stage |
| Web / mobile | Optional, undecided; Proa is a candidate for a web side |
