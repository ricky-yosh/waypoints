# Agent-led workflows and desktop onboarding

Product decision: the user's existing harness is the primary conversation
surface. Its agent creates and manages Waypoints workflows and coordinates
other agents through the existing ACP-backed backend. The desktop app is an
on-demand visual companion for following construction, reviewing code, and
intervening.

This document specifies the implemented behavior and the remaining platform
limits. See [desktop usage](05-ui-and-diffs.md) and
[backend/CLI usage](11-backend-cli.md) for usage details.

## Why use Waypoints

A representative user request:

> We are working on a large feature that will need iteration. I want to see
> it grow from a skeleton into a fleshed-out feature, so use Waypoints.
> Break the work into reviewable pieces and coordinate the implementation agents.

The workflow should make both construction and code history easier to inspect:

- Begin with a useful skeleton, then flesh it out through scoped assignments.
- Make dependencies and parallel work explicit instead of hiding them in chat.
- Let the user inspect each assignment's code and exact native revisions.
- Use Waypoints' jj-backed changes and successful-turn snapshots to keep
  reviewable history as work evolves.

A node is one evolving jj change, not necessarily one Git commit or one agent
turn. Do not imply automatic Git publication, merging, or a commit for every
visible progress update. Combining sibling results and publishing history
remain explicit operations under the existing contracts.

## Primary workflow

1. The user asks their harness agent to use Waypoints from a repository.
2. The coordinating agent inspects the project using Waypoints' existing
   repository discovery and readiness checks before creating work. It selects
   an explicit workflow base rather than guessing one.
3. The agent creates the workflow, scoped assignments, native changes, and
   workspaces through supported CLI/client operations. It starts and monitors
   eligible worker agents through ACP; the harness agent is the coordinator,
   not implicitly another worker session.
4. The coordinator requests the desktop app to open the relevant project and
   workflow. The visual reflects backend-owned state; opening it does not
   recreate the workflow or launch duplicate runs.
5. The user continues directing work from the harness and uses the canvas,
   output, diffs, and history as a shared reference. Existing app run controls,
   assignment editing, and permission responses remain useful interventions.
6. Closing the visual does not cancel backend-owned runs. Reopening reveals
   current state rather than starting the work again.

Waypoints is not a new conversational harness or a replacement agent loop.
The coordinating agent uses Waypoints tools/CLI; Waypoints' backend takes the
ACP client role and manages the worker subprocesses.

## Repository checks are part of the workflow

- Reject a non-repository folder with an actionable explanation before mutation.
- A Git repository alone is not sufficient under today's native contracts:
  creation requires the existing jj repository/workspace and base checks too.
- Inspect and explain missing prerequisites; do not silently initialize Git/jj,
  select a guessed base, repair native state, or bypass readiness checks.
- Preserve structured backend errors, conflicts, busy states, and recovery
  safeguards. A visually empty workflow is not evidence that native state is safe.

See [frontend-facing backend support](25-backend-frontend-readiness.md),
[workflow bases](16-workflow-bases.md), and
[run lifecycle](13-agent-run-lifecycle.md).

## Opening and live synchronization

The selected contract is:

```sh
waypoints app open --project PATH [--workflow ID] [--json]
```

It uses a private Unix control socket, separate from the workflow backend
socket, at `$XDG_STATE_HOME/waypoints/desktop.sock` or
`~/Library/Application Support/waypoints/desktop.sock` on macOS. Set
`WAYPOINTS_APP_SOCKET` to choose another absolute path. The CLI validates that
the folder exists and, when `--workflow` is given, asks the backend client
whether that workflow exists and belongs to the project. A running app receives
the request on the socket and comes forward; otherwise the CLI starts
`waypoints-app` (or `WAYPOINTS_APP_EXECUTABLE`) with the same selection and
waits for it to accept. Opening is idempotent and separate from creation or
execution. If no display starts, the CLI reports the failure and the backend's
headless work is untouched.

- Reuse/focus an existing app instance where appropriate rather than opening
  duplicates or asking the user to find the newly created workflow manually.
- Opening the visual is separate from creation and execution. If no desktop
  display is available, return a clear result without undoing headless work.
- Workflows created or changed by the coordinator become visible without
  repeated manual Refresh actions: the app quietly refreshes the workflow list,
  selected graph, and node run statuses every two seconds. Manual Refresh remains
  available. A failed background refresh reports **Reconnecting** and the error
  in the project panel while the loaded canvas and drafts stay visible.
- Selection and unsaved drafts stay safe when externally changed state arrives.
  A clean draft follows a newly saved prompt; a dirty draft is kept with an
  explicit warning.
- Do not automatically start worker runs merely because a workflow is opened.
  Runs started through the existing CLI/client paths appear as their backend
  state changes.

## App entry points and empty states

Users can still open projects directly. Workflow creation is intentionally
agent-led; there is no required in-app workflow creation form or second chat.
Distinguish these states instead of showing the same generic empty canvas.

### No project open

Primary message: **Open a project to see its workflows.**

Provide a large folder drop target with **Drop your project folder here** and
an **Open project…** button as an equivalent keyboard-accessible action.
Both routes use the same project validation and error handling. Dropping a
folder opens/selects a project; it does not create a workflow or initialize a
repository. Reject unsupported or ambiguous drops without choosing silently.

An optional decorative background can show nodes appearing, disappearing, and
reconnecting to communicate the product. It must:

- Be clearly illustrative, not appear to represent real work or run status.
- Never steal focus, capture drops, or obscure the primary action.
- Be hidden from accessibility and respect reduced motion with a static scene.
- Stop requesting animation frames when hidden; avoid unnecessary idle work.

The animation is polish, not a prerequisite for functional onboarding.

### Project open, no workflows

Primary message: **To create a workflow, ask your AI agent to do so using Waypoints.**

Explain: **In your agent harness, open this repository and paste this prompt.**
Show the selectable prompt and an adjacent **Copy prompt** action:

```text
Use waypoints to make a workflow and create this feature
```

Tell the user to add the feature description before sending. Copying must
preserve the displayed text and provide accessible success/failure feedback;
it must not send a message or start work automatically.

Offer another project choice without implying that opening a folder creates
workflows. Distinguish an empty result from loading, disconnection, or an error.

### Persistent create-workflow affordance

Keep **New workflow…** available for the selected project even when workflows
already exist. A plus icon is acceptable with an accessible name and tooltip.
It opens the same instructions and copyable prompt, not a creation form or
an automatically executing action. Do not hijack the separate Add/Open project
control for this purpose.

Keep the prompt/instructions in one implementation source so the empty state
and persistent affordance cannot drift. Preserve the current workflow and draft
when showing or dismissing these instructions.

## Acceptance criteria

- From a repository, the harness coordinator can create scoped work through
  supported operations and coordinate ACP workers without requiring app-first
  setup or a conversation in the desktop app.
- Non-repository and Git-only/not-ready projects fail with actionable guidance,
  without implicit repository initialization or native repair.
- An agent-open request reveals the intended workflow without duplicate work;
  display failure leaves headless operations intact.
- No-project and no-workflow states are distinct. Folder drop and keyboard
  project opening share validation.
- The exact prompt is visible/selectable/copyable in the no-workflow state and
  persistent New workflow affordance. Copying never starts work.
- External creation/progress updates appear while the app is open, preserving
  drafts and exposing connection failures.
- Onboarding works with keyboard navigation, larger text, light/dark themes,
  accessibility, and reduced motion. Decorative animation conveys no fake state.

Implementation sequencing: establish the coordinator/open/synchronization
contracts and functional empty states before adding background motion. This
decision does not by itself authorize changes to backend safety semantics,
automatic graph scheduling, Git publication, or an in-app agent harness.
