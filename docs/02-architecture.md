# Architecture

## Layers

1. **GPUI app (UI layer):** plan blocks, node canvas, phase view (markdown and diffs), run stream.
2. **Core crate (no UI):** phase executor, phase state machine, store.
3. **Integrations:** ACP client, jj workspaces, "Open in Zed".
4. **Outside the app:** agent adapters (Claude, Codex, Gemini, ...) and the user's repo (jj colocated with git).

Keep the core crate free of UI code so it can be built and tested with a plain CLI first.

## Sessions: when does a node keep an ACP session?

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
- Optional event log of streamed updates for replay.

## Re-running and "nodes extending onward"

Treat each re-run of a phase as a new revision of the same node. The canvas can show a short stack of versions, with downstream nodes attached to the version they were built on. jj change IDs stay stable across rewrites, so a node can follow its change.

## Context fallback

Because session reuse is optional per agent, always keep a working fallback: start a fresh session whose first prompt contains the plan text plus a summary of earlier phases.
