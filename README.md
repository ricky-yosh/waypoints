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
blocked. The desktop app provides an initial readonly workflow browser.

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

## Try the desktop browser

Start the backend with a workflow command, for example
`cargo run -p waypoints-cli -- workflow list --project .`, then run
`cargo run -p waypoints-app`. Choose **Add project…** to open a folder in the
Projects sidebar. Multiple projects can stay open; expand a project and select
an active workflow to inspect its saved nodes and dependencies. Each project has
its own Refresh/Retry action; toolbar Refresh targets the selected workflow's project,
including changes made through the CLI. **Remove from sidebar** only removes the
UI entry: it never deletes project files or saved workflows.

Drag the divider to resize the sidebar, or use **Show/Hide Projects** in the View
menu. On macOS, Command-O adds a project, Command-B toggles the sidebar, and
Command-R refreshes the selected project (Control equivalents on Linux).
Open folders, expanded sections, selection, sidebar layout, and window bounds are
remembered in `waypoints/workspace.json` under the platform configuration directory
(`~/Library/Application Support` on macOS; `$XDG_CONFIG_HOME` or `~/.config` on Linux).
Unavailable saved folders remain visible for retry or removal. Invalid or newer
preferences files produce a warning and are not overwritten; repair or move the
file to allow preferences to save again.

The browser is readonly, does not automatically start the backend, and uses the
CLI's default database and socket paths (including `XDG_STATE_HOME`); custom CLI
`--database` and `--socket` options are not yet exposed in the GUI. An interrupted
dependency operation is shown because saved edges may not reflect native state.

### Develop with native hot reload

The desktop app uses GPUI Kit's **GPUI Fast** backend for retained rendering and
layout, including in tests. Application code continues to use `gpui_kit`.
The local patches preserve the AccessKit window-close fix for GPUI Fast;
see [`vendor/README.md`](vendor/README.md).

The desktop UI uses experimental native Rust hot patching for development.
From the repository root, run:

```sh
./dev
```

This launches Dioxus CLI with the `hot-reload` feature and patches code in the
running app rather than restarting it. Edit a label, spacing, or layout in a
`render` method and save to see the change in the existing window. Restart the
development session after changing struct fields, constructors, startup logic,
or persisted-state formats, or if a patch fails.

Install the matching CLI once:

```sh
cargo install dioxus-cli --version 0.7.10 --locked
```

The script uses `${CARGO_HOME:-$HOME/.cargo}/bin/dx` explicitly because another
program named `dx` may be on `PATH`. Set `DIOXUS_DX` to use a different Dioxus
installation. Extra arguments are forwarded; `./dev --help` shows the CLI options.
It always runs from the workspace root and selects `waypoints-app`, avoiding a
Dioxus 0.7.10 default-member path bug when launched from a nested directory.

The opt-in feature is forwarded through the local `vendor/gpui-kit` patch to
GPUI Fast's Subsecond integration only.
Normal `cargo run -p waypoints-app` builds do not
enable hot reload. The backend still needs to be started separately as described
above; UI code hot reload does not refresh workflow data.

### Dependency update PRs

[`renovate.json`](renovate.json) configures a GPUI-only Renovate trial: grouped
Cargo updates on Mondays before 06:00 UTC, with manual merging. Other dependencies,
vendored manifests, and general lockfile maintenance are excluded from this trial.

To activate it, install the [Renovate GitHub app](https://github.com/apps/renovate)
for `ricky-yosh/waypoints`, then merge this configuration into the default branch.
Review any onboarding PR or Dependency Dashboard setup errors. Adding the
configuration locally does not activate the hosted bot.

The GPUI family currently uses local hot-reload and AccessKit compatibility
patches. Renovate can propose new upstream versions, but it cannot refresh the
vendored sources automatically. Follow [`vendor/README.md`](vendor/README.md)
before merging an update PR. Resolve any lockfile errors, run tests and lint
checks, and verify native Settings close/reopen and `./dev` on macOS. A dependency
PR alone is not evidence that the window-close crash is fixed.

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
| [`25-backend-frontend-readiness.md`](docs/25-backend-frontend-readiness.md) | Repository/base discovery, explicit stale-workspace updates, structured failures, and responsive live run controls |

## Decisions so far

| Topic | Decision |
|---|---|
| Agent integration | ACP client, one adapter per agent, no custom agent loop |
| Code history | One evolving jj change per node, many exact revisions; workspaces allocated on demand |
| Native feedback | Reflect jj changes/conflicts; no inferred downstream-context-staleness state for v1 |
| Diff viewing | Headless exact-revision review implemented; inline GUI review and editor handoff pending; no custom multibuffer |
| UI | GPUI Kit (`gpui-kit`); initial readonly desktop browser; see [current stack](docs/06-stack-and-storage.md#gpui) |
| Storage | Backend-owned Turso with plain SQL; SQLite migration and backup procedures are not implemented |
| Prototype | React/TypeScript/React Flow is an optional design experiment, not a required build stage |
| Web / mobile | Optional, undecided; Proa is a candidate for a web side |
