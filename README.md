# Waypoints: plan-driven agent workflows

Status: early implementation. Core/backend/CLI support
persistent rooted workflows, native jj changes and on-demand workspaces, ACP runs
and continuation, automatic successful-turn snapshots, same-change run/commit restoration
and explicitly confirmed same-workflow downstream rebasing,
metadata undo/redo, native reparenting, multi-parent native creation,
native deletion with retained workspace folders,
read-only native code review, paginated revision history and restore-impact inspection,
an informational upstream-revision hint, and undoable prompt-only move/delete.
Explicit recovery handles clear interrupted outcomes; ambiguous states remain
blocked. The macOS desktop app is validated; Linux/Windows are not yet verified.
It provides a project/workflow sidebar, canvas dependency graph, editable
assignments, live agent output and controls, exact patch review, and native
revision history. Workflow, native-change, and workspace provisioning
still use the CLI.

**V1 reflects native jj revisions and conflicts.** Users decide what changed
assumptions require review or rerunning; a separate downstream-context-staleness
badge or execution gate is not required. Physical-checkout diagnostics and
restore-specific fresh-session safeguards remain in place. See the
[scope decision](docs/02-architecture.md#decision-reflect-native-jj-changes-not-inferred-context-staleness).

Start with the [current core/backend status and documentation map](docs/20-core-backend-status.md)
for implemented boundaries, remaining discussion items, and links to detailed contracts.

## Intended workflow: agent-led, visually reviewable

Ask your agent in its existing harness to use Waypoints for a large, iterative
feature. The agent creates a workflow, breaks work into scoped assignments, and
coordinates ACP workers. The desktop canvas is the visual companion for seeing
the feature grow from skeleton to implementation and reviewing jj-backed code
history; the harness remains the primary conversation surface.

Users can also open projects directly. Onboarding directs workflow creation back
to the harness with this copyable prompt:

```text
Use waypoints to make a workflow and create this feature
```

Add the feature description before sending. Targeted app opening, live external
synchronization, and the distinct empty states are implemented. See
[agent-led workflows and onboarding](docs/28-agent-led-workflows.md).

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

## Try the desktop app

Start the backend with a workflow command, for example
`cargo run -p waypoints-cli -- workflow list --project .`, then run the
canonical Rune desktop app:

```sh
cargo run -p waypoints-app
```

The first launch shows a folder drop target and **Open project…** instead of a
fake graph. A project with no workflows shows the exact copyable harness prompt;
the HUD and Projects panel keep a **New workflow** control for the same
instructions. Ask your harness agent to create work through the CLI/client, then
have it reveal the saved workflow:

```sh
waypoints app open --project . --workflow 1
```

This focuses a running app or starts one, without creating or running anything.
The app refreshes externally created workflows and run progress every two
seconds and reports **Reconnecting** if a background refresh fails.

Use **Add** in the Projects panel to choose a project folder. The panel keeps a
Rune-only project list and the selected project/workflow; remove an entry with
the `×` button, which never deletes project files. The graph is canvas-first:
drag to pan, pinch or ⌘-scroll to zoom, click to select, drag a card to move its
presentation position, and double-click or press Return on a focused card to
open the workbench.

The workbench provides Assignment, Output, Changes, and History tabs. Edit or
preview the saved assignment, start/continue/cancel runs, reply to permissions,
review exact patches, and browse saved runs or native revisions. Hover or press
Shift+Return for a card preview, Escape closes the topmost layer, and the
Workbench/Settings views return focus to their opener. `Cmd+1–4`, `Cmd+R`,
`Cmd+B`, `Cmd+O`, `Cmd+,`, and the other shortcuts are listed in Settings.

On macOS the app installs native menus (Settings, Add Project, Close Window,
Show Projects Sidebar, Refresh Project) plus the standard Edit and Window menus.
Rune's pinned menu API is macOS-only; Linux/Windows use the in-app controls.
Window size/position restore and clamp to connected displays. Rune preferences
are stored separately at `waypoints/rune-workspace.json` under the platform
state directory (`~/Library/Application Support` on macOS,
`$XDG_STATE_HOME/waypoints` or `~/.local/state/waypoints` elsewhere). Invalid or
newer files are left untouched. No GPUI preference file is read or written.

The backend must be started separately as described above; the app can also
start it when a saved project is selected. Workflow/native-change/workspace
provisioning and code restoration remain CLI operations. See
[Working in the desktop app](docs/05-ui-and-diffs.md) for controls and limits.

### Develop

```sh
./dev
```

`./dev` launches `cargo run -p waypoints-app` from the workspace root. Native
hot reload is not part of the Rune build; restart after changing startup or
persisted-state formats. Use `cargo test -p waypoints-app --locked` and the
headless commands in [Rune validation](docs/port-info/validation.md) for the
repeatable checks.

### Dependency updates

[`renovate.json`](renovate.json) enables ordinary Cargo dependency updates on
Monday mornings (manual merge, no lockfile maintenance). The former GPUI vendor
patch/update workflow has been removed with the GPUI frontend.

## One line

Plan branching coding-agent work around evolving jj changes, inspect exact
revisions, and explicitly choose what to run or revisit.

## Short description

Product direction, not a list of implemented GUI/backend capabilities:

Waypoints turns a plan into a graph of assignments. A node can own one evolving
jj change with many exact revisions, and its agent work can use fresh sessions or
explicit continuation over ACP. Review diffs, branch parallel work, revisit an
earlier revision, and see jj's resulting changes and conflicts. The headless
implementation is Rust; the desktop client uses Rune and Rune Kit. Codex is the
manually validated adapter, not a guarantee that every ACP agent is supported.

## Documentation

| File | What it covers |
|---|---|
| [`20-core-backend-status.md`](docs/20-core-backend-status.md) | Start here: current capability inventory, documentation map, and unprioritized discussion backlog |
| [`01-concept.md`](docs/01-concept.md) | Product direction, terminology, and non-goals |
| [`02-architecture.md`](docs/02-architecture.md) | Ownership and scope decisions; explicitly labeled future sketches |
| [`03-acp-and-agents.md`](docs/03-acp-and-agents.md) | Implemented ACP boundary versus protocol and adapter research |
| [`04-jj-snapshots.md`](docs/04-jj-snapshots.md) | Evolving changes, exact revisions, native conflicts, and restoration |
| [`05-ui-and-diffs.md`](docs/05-ui-and-diffs.md) | Current desktop workflow, node workbench, exact diff presentation, and UI validation |
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
| [`25-backend-frontend-readiness.md`](docs/25-backend-frontend-readiness.md) | Repository/base discovery, explicit stale-workspace updates, structured failures, and responsive live run controls |

## Decisions so far

| Topic | Decision |
|---|---|
| Agent integration | ACP client, one adapter per agent, no custom agent loop |
| Code history | One evolving jj change per node, many exact revisions; workspaces allocated on demand |
| Native feedback | Reflect jj changes/conflicts; no inferred downstream-context-staleness state for v1 |
| Diff viewing | Headless and GUI exact-revision review implemented; editor handoff pending; no custom multibuffer |
| UI | Rune and Rune Kit; canvas graph and node workbench backed by the shared API; see [desktop workflow](docs/05-ui-and-diffs.md) and [current stack](docs/06-stack-and-storage.md#rune) |
| Storage | Backend-owned Turso with plain SQL; SQLite migration and backup procedures are not implemented |
| Prototype | React/TypeScript/React Flow is an optional design experiment, not a required build stage |
| Web / mobile | Optional, undecided; Proa is a candidate for a web side |
