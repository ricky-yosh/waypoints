# Architecture

This page records architecture and ownership decisions. The
[core/backend status map](20-core-backend-status.md) is the current capability
inventory; [backend/CLI](11-backend-cli.md) is the transport and operation reference.
Design sketches below are explicitly separated from implemented behavior.

## Layers

1. **Headless core library:** workflow operations, phase prompts and associations, storage, and jj integration. It has no GUI dependency and can be tested without opening a window.
2. **Local application backend:** a running process hosting the core and exposing a shared JSON-RPC API.
3. **Clients:** the GPUI desktop app and a CLI both use that API. Neither implements a separate workflow engine.
4. **Outside Waypoints:** existing agent harnesses and the user's native jj repository. ACP integration launches a trusted user-installed executable; Waypoints does not implement the agent loop.

## Architecture decision: local backend, GUI, and CLI

**Decision:** Waypoints is a local desktop tool with a UI-independent core, a shared local backend, and GUI and CLI clients. The CLI is the agent-facing surface for existing harnesses with terminal access. **MCP is out of scope; do not add an MCP adapter or require `gpui-mcp`.**

This decision supersedes earlier CLI-as-UI-implementation wording and MCP recommendations in the tooling research. Borrowing the client/backend pattern from Codex does not mean building a conversation manager.

```text
Desktop GUI ---+
               +--- Local JSON-RPC backend --- Headless core --- jj + workflow metadata
CLI -----------+
```

- **RPC (remote procedure call)** lets one process request an operation from another. Here both processes run on the same machine; "remote" does not mean hosted.
- **JSON-RPC** supplies structured requests, responses, and notifications. It does not itself provide persistence or reconnection behavior.
- **Shared operations:** both clients query and mutate workflows through the backend. It coordinates Waypoints-initiated writes and jj operations; jj remains the source of truth for native version-control state.
- **Agent interface:** the CLI supports structured JSON output, noninteractive invocation, clear errors and exit codes, explicit project/workflow selection, and bounded event polling. A continuous terminal renderer is not implemented.
- **Persistence:** save workflow prompts, links, and native-change associations so they survive restarts. Conversations and transcripts remain in the existing harness; do not store them in Waypoints.
- **Updates:** current clients poll run status/events and fetch current saved state after reconnect. No subscriptions, durable notification log, or notification replay are required. Native refresh reports current/missing/divergent revisions and conflicts, not a full ancestry assessment. General reconciliation is not implemented; inferred context-staleness tracking is outside v1 scope under the decision below.
- **Human-paced execution:** a workflow connection does not automatically run the next phase.
- **UI debugging is separate:** the native GPUI inspector helps develop the GUI. Agents do not need to click the GUI to perform workflow operations. Screenshots, when useful for visual verification, need not be stored persistently.

**Implemented background-run checkpoint:** ACP tasks own their subprocesses outside
short request handlers. Start returns a run ID; clients poll bounded transient
events and explicitly answer permissions or cancel. Database writes remain
backend-owned and serialized. Uncertain runs retain reservations after a crash;
no prompt is automatically replayed. See [scope and validation](13-agent-run-lifecycle.md).
This uses polling rather than subscriptions for the initial update transport.

**Implemented topology:** one backend owns a configured database/socket pair,
using local Unix-domain IPC on macOS/Linux, not a public HTTP endpoint. A database
can contain workflows for multiple canonical project folders. The CLI auto-starts
or reuses the backend; the desktop app connects to an already-running backend.
Lifetime locks prevent competing owners. Access is restricted
to the owning user. Windows named pipes and automatic per-repository backend
selection are not implemented. See [lifecycle](11-backend-cli.md#lifecycle).

**Implemented direction: headless core with CLI and desktop clients.** Workflow persistence,
branching, native editing, and ACP execution use the shared backend. The desktop
provides graph navigation, assignment editing, run controls, output, review, and
history; provisioning and code restoration still use the CLI. See the
[desktop workflow](05-ui-and-diffs.md) for the current UI boundary. A separate
project registry is not implemented: project identity is the canonical folder.
Workflows are sidebar items grouped by project; do not introduce a
separate application-level "session" entity. Agent session references belong to
existing harnesses. See [acceptance and research](07-spikes-and-build-order.md).

## Workflow model and native history

**Implemented for based workflows:** the graph represents native code dependencies
rooted at a fixed saved base. Node creation attaches metadata atomically; later
native creation uses that dependency. Checked reparenting and deletion change jj
and reconcile the graph with an acknowledged undo boundary. Deletion can
deregister a ready saved workspace while retaining all its files. Filesystem
cleanup and broader reconciliation remain pending.
[The native dependency decision](15-native-dependency-editing.md) supersedes the
planning-only model. Legacy unbased commands retain that older behavior; no old
graph is silently adopted.

- A project identifies a local project folder/repository and groups workflows. The CLI selects projects and workflows explicitly; the desktop presents that organization in its sidebar.
- A workflow contains prompt nodes and saved connections, with branching supported from the start. See the [interfaces and parallel implementations example](01-concept.md#project-workflow-and-node-example).
- A node represents a prompt/assignment associated with an evolving jj change, not a frozen commit or a single agent turn. A change ID follows evolving work; commit IDs identify exact code revisions. Resolve native divergence explicitly rather than assuming a change ID always identifies one current commit.
- Keep workflow metadata and native jj state as distinct sources with checked alignment, not unrelated dependency graphs. Do not guess prompt associations from native ancestry. External disappearance of a change retains its saved node/association as missing; explicit Waypoints deletion instead abandons native work and removes the node from the active graph while preserving historical references.
- Browsing native revisions and continuing work from an old revision are separate operations. Validate jj commands and retention before promising indefinite availability or a particular restore/branch behavior.
- Prompt-edit undo history is distinct from native code history. Associating an exact historical prompt with a code revision requires an explicit recorded reference; do not infer that correspondence.
- Separate workspaces are needed for simultaneous editing of different code states, not for every idle node or historical revision. Allocate working directories on demand; two concurrent editors/agents must not share a directory for different nodes.
- Workflow metadata undo/redo remains chronological even though the workflow and native code graph can branch. Neither undoing a prompt nor browsing code history rewinds an agent conversation.

## Architecture decision: persistent workflow undo/redo

Implement a small, project-owned undo/redo system rather than adopting a dedicated undo crate or generic command framework. Reuse the project's existing storage and serialization dependencies. The required customization is in workflow semantics and transactional persistence, not a general-purpose undo engine.

- **Scope:** workflow renaming, node creation with its initial attachments, prompt edits, legacy unbased connection creation, and eligible prompt-only move/delete. Native reparenting and deletion establish an explicit undo boundary rather than becoming reversible metadata commands. Code changes, jj operations, agent execution, and conversations are outside this history; let jj handle code recovery.
- **Ownership:** one chronological history per canonical project folder, shared by clients and workflows in that folder. This does not automatically group different subfolders of one repository. The backend serializes edits, undo, and redo.
- **Records:** represent supported edits as explicit, versioned data with enough before/after information to reverse and reapply them. Do not persist executable closures. Redo restores recorded values and identities rather than rerunning arbitrary side effects.
- **Atomicity:** commit each workflow mutation and its history record/cursor update in the same database transaction. Undo and redo are transactional too; failures must not advance the cursor.
- **Persistence:** retain history and its current position across application restarts. Load saved workflow state directly rather than reconstructing it by replaying all commands. This edit history is not event sourcing or a durable client-notification log.
- **Branching:** a new saved edit after undo discards the redo tail, atomically with the new edit. Selective undo and branching histories are not initial requirements.
- **Grouping:** one meaningful metadata action is one undo step. Saving a prompt creates one step; typing retains normal local input undo. The revised native node deletion policy is outside ordinary metadata undo; it abandons the associated change and reconciles dependent graph links.
- **Feedback:** expose the next metadata action, such as "Edit prompt," with its stable edit ID and workflow ID. Native deletion is not an undoable metadata action.
- **Upgrades:** migrate stored history formats or explicitly invalidate incompatible history without deleting the current workflow; never silently misinterpret old records.

**Implemented:** the supported metadata actions above have transactional,
persistent undo/redo through the backend and CLI.
History is shared across workflows in one canonical project folder, with explicit
expected edit IDs to reject stale clients. Native/run references block undoing a
node's creation rather than being deleted. Existing data is a non-undoable baseline.
Disconnection, ordering, and layout remain future slices. Native reparenting and
deletion have separate undo boundaries and explicit recovery. Code and agent
conversation state are never part of this history. See [commands and validation](14-workflow-history.md).

**Native dependency editing:** `node reparent` and `node delete` require explicit
undo-boundary acknowledgement. Workflow Undo cannot cross that boundary or reverse
native code. General connect/disconnect behavior remains a discussion item;
legacy `connection create` has not acquired rebase semantics. See the
[history policy](14-workflow-history.md#decision-acknowledge-future-code-changing-connections).

## Sessions: when does a node keep an ACP session?

Fresh runs and explicit same-session continuation are implemented in the
[run lifecycle](13-agent-run-lifecycle.md). Session references point to
harness-owned conversations, not a Waypoints chat or transcript store.

- Give each running phase its own agent process and ACP session.
- Shut the process down when the phase ends. Idle nodes hold nothing open.
- Save the agent's session ID for explicit loading when supported. Do not silently start fresh if loading fails.
- A successful turn enters finalization after process cleanup, then completes with a separate snapshot outcome. This is not human code-review approval; workflow review gates are not implemented.

## Decision: reflect native jj changes, not inferred context staleness

For v1, jj is the source of truth for code state. Surface its revisions,
rewrites, divergence and conflict flags directly; do not build a separate
conflict detector or infer that an agent's reasoning is invalid.

- Users infer changed assumptions from code history and dependencies, and
  explicitly choose what to review, test or rerun. A clean rebase is not proof
  of behavioral correctness, but does not require an additional warning state.
- Do not add a separate downstream "upstream changed" or "stale context" badge,
  acknowledgement step, or execution gate for v1. General causal/transitive
  context tracking is not a prerequisite for the GUI or v1 completion.
- Keep stale **physical checkout** diagnostics distinct from native conflicts;
  they describe files needing explicit reconciliation, not agent assumptions.
- Retain existing restore-specific fresh-session barriers and other operation
  safety checks. This decision does not relax them or automatically update files.
- The existing best-effort CLI upstream-input hint may remain informational;
  it is not an invalidation policy or a requirement for a new GUI indicator.
- Never automatically rerun tasks, inject context or resolve conflicts.

This supersedes earlier suggestions that general downstream context-staleness
tracking must be implemented before the GUI or treated as a v1 completeness gap.
Broader native graph reconciliation remains a separate discussion.

## Phase states (product sketch, not the current API)

draft -> ready -> running -> awaiting approval -> done

Plus: failed and conflicted (jj reports a conflict). The earlier proposed
downstream `stale` phase state is not part of v1; see the decision above.

Current run states, native conflict flags, graph connectivity, and workspace
readiness are separate values, not this single phase state machine.
`awaiting_permission` means an ACP tool decision, not a code-review gate.
The sketched review-approval states are not approved v1 requirements.
See the [run-state contract](13-agent-run-lifecycle.md#lifecycle-and-ownership).

## Data to store (product sketch)

This sketch includes future fields; consult the feature guides for current records.
It does not claim that timestamps, model/capability provenance, layout, or
workflow review state are currently captured.

- Plans, phases, points (the text and ordering).
- Runs or revisions of a phase (a node can have several versions as you iterate).
- For each run: agent name, ACP session ID, agent capabilities, jj change ID, workspace path, status, timestamps.
- Versioned workflow undo/redo records and their current position, saved atomically with workflow edits.
- Client notifications are transient; a durable event log or transcript archive is not part of the initial architecture.

## Re-running and "nodes extending onward"

Refining a phase can evolve the same node's native jj change through multiple commit revisions. Do not assume every turn creates a new child change or an application-owned code snapshot. Preserve exact native revision references where needed to identify inputs. Users explicitly choose whether to continue a change or create an alternative; follow native jj ancestry and surface divergence and conflicts, not inferred downstream context staleness. See [jj snapshots](04-jj-snapshots.md).

## Unavailable saved context

Session reuse is optional per agent. An explicit continuation must report unsupported
loading or missing history, never silently start fresh. Users may explicitly start
a fresh session with the saved node assignment. Seeding a different phase with plan
text or earlier-phase context is a separate future action, not a load-failure fallback.
