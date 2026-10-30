# Architecture

## Layers

1. **Headless core library:** workflow operations, phase prompts and associations, storage, and jj integration. It has no GUI dependency and can be tested without opening a window.
2. **Local application backend:** a running process hosting the core and exposing a shared JSON-RPC API.
3. **Clients:** the GPUI desktop app and a CLI both use that API. Neither implements a separate workflow engine.
4. **Outside Waypoints:** existing agent harnesses and the user's repository (jj colocated with git). Harness integration can follow the first manual workflow.

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
- **Agent interface:** the CLI should support structured JSON output, noninteractive invocation, clear errors and exit codes, explicit repository selection, and waiting for or streaming relevant updates.
- **Persistence:** save workflow prompts, links, and native-change associations so they survive restarts. Conversations and transcripts remain in the existing harness; do not store them in Waypoints.
- **Updates:** begin with state-change notifications that tell clients to refresh. Reconnecting clients fetch a current state snapshot; no durable event log or event replay is required initially. External jj changes are reconciled on refresh.
- **Human-paced execution:** a workflow connection does not automatically run the next phase.
- **UI debugging is separate:** the native GPUI inspector helps develop the GUI. Agents do not need to click the GUI to perform workflow operations. Screenshots, when useful for visual verification, need not be stored persistently.

Recommended starting topology: one backend per open repository, shared by its clients, using local IPC (Unix-domain sockets or Windows named pipes), not a public HTTP endpoint. Restrict access to the owning user. A GUI or CLI may start or connect to it; an always-running background service is not required. Backend lifetime and IPC details still need implementation validation.

**Current implementation priority: headless core and CLI first; GUI later.** Establish project-folder registration, multiple branching workflows, node editing, and persistence through the local backend and CLI before implementing the node view. Workflows are the sidebar items grouped by project; do not introduce a separate application-level "session" entity. Agent session references belong to existing harnesses. AI execution integration is not a prerequisite. This supersedes the earlier UI-first milestone, without changing the shared-backend architecture. See [build order](07-spikes-and-build-order.md).

## Workflow model and native history

- A project identifies a local project folder/repository and groups workflows. The CLI selects projects and workflows explicitly; the later GUI presents that organization in its sidebar.
- A workflow contains prompt nodes and saved connections, with branching supported from the start. See the [interfaces and parallel implementations example](01-concept.md#project-workflow-and-node-example).
- A node represents a prompt/assignment associated with an evolving jj change, not a frozen commit or a single agent turn. A change ID follows evolving work; commit IDs identify exact code revisions. Resolve native divergence explicitly rather than assuming a change ID always identifies one current commit.
- Keep workflow links distinct from jj parent relationships. Reflect manual repository changes without guessing prompt associations. A missing native change does not delete the saved node; mark the association missing and require an explicit action to create or select a replacement.
- Browsing native revisions and continuing work from an old revision are separate operations. Validate jj commands and retention before promising indefinite availability or a particular restore/branch behavior.
- Prompt-edit undo history is distinct from native code history. Associating an exact historical prompt with a code revision requires an explicit recorded reference; do not infer that correspondence.
- Separate workspaces are needed for simultaneous editing of different code states, not for every idle node or historical revision. Allocate working directories on demand; two concurrent editors/agents must not share a directory for different nodes.
- Workflow metadata undo/redo remains chronological even though the workflow and native code graph can branch. Neither undoing a prompt nor browsing code history rewinds an agent conversation.

## Architecture decision: persistent workflow undo/redo

Implement a small, project-owned undo/redo system rather than adopting a dedicated undo crate or generic command framework. Reuse the project's existing storage and serialization dependencies. The required customization is in workflow semantics and transactional persistence, not a general-purpose undo engine.

- **Scope:** saved prompt edits, phase creation/deletion, workflow connections, ordering, and layout. Code changes, jj operations, agent execution, and conversations are outside this history; let jj handle code recovery.
- **Ownership:** one chronological history per repository, shared by GUI and CLI clients. The backend serializes edits, undo, and redo.
- **Records:** represent supported edits as explicit, versioned data with enough before/after information to reverse and reapply them. Do not persist executable closures. Redo restores recorded values and identities rather than rerunning arbitrary side effects.
- **Atomicity:** commit each workflow mutation and its history record/cursor update in the same database transaction. Undo and redo are transactional too; failures must not advance the cursor.
- **Persistence:** retain history and its current position across application restarts. Load saved workflow state directly rather than reconstructing it by replaying all commands. This edit history is not event sourcing or a durable client-notification log.
- **Branching:** a new saved edit after undo discards the redo tail, atomically with the new edit. Selective undo and branching histories are not initial requirements.
- **Grouping:** one meaningful action is one undo step. Saving a prompt creates one step; typing retains normal local input undo. Deleting a phase includes its removed workflow links in the same step and does not abandon its jj change.
- **Feedback:** expose what undo/redo will do, such as "Undo delete phase," to both clients.
- **Upgrades:** migrate stored history formats or explicitly invalidate incompatible history without deleting the current workflow; never silently misinterpret old records.

## Sessions: when does a node keep an ACP session?

The following ACP execution sketches are for later harness integration, not requirements for the first slice. Any session references point to harness-owned conversations; they do not imply a Waypoints chat or transcript store.

- Give each running phase its own agent process and ACP session.
- Shut the process down when the phase ends. Idle nodes hold nothing open.
- Save the agent's session ID in the store so the session can be resumed later if the agent supports it.
- A phase is finished when the prompt turn returns with a stop reason. The state machine then moves it to done, or to awaiting approval if a gate is set.

## Phase states

draft -> ready -> running -> awaiting approval -> done

Plus: failed, stale (an upstream phase changed), conflicted (jj reports a conflict).

## Data to store (sketch)

- Plans, phases, points (the text and ordering).
- Runs or revisions of a phase (a node can have several versions as you iterate).
- For each run: agent name, ACP session ID, agent capabilities, jj change ID, workspace path, status, timestamps.
- Versioned workflow undo/redo records and their current position, saved atomically with workflow edits.
- Client notifications are transient; a durable event log or transcript archive is not part of the initial architecture.

## Re-running and "nodes extending onward"

Refining a phase can evolve the same node's native jj change through multiple commit revisions. Do not assume every turn creates a new child change or an application-owned code snapshot. Preserve exact native revision references where needed to identify inputs. Users explicitly choose whether to continue a change or create an alternative; follow native jj ancestry and surface divergence, conflicts, and stale downstream context. See [jj snapshots](04-jj-snapshots.md).

## Context fallback

Because session reuse is optional per agent, always keep a working fallback: start a fresh session whose first prompt contains the plan text plus a summary of earlier phases.
