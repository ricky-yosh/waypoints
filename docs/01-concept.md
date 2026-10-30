# Concept

## The problem

Work with coding agents often means chaining prompts by hand: run one, wait and watch the output (watching is useful), copy results into the next prompt, and lose track of which results still hold when something upstream changes. Delta (from the Zed team) makes single threads feel fluid, but the plan that decides which threads exist, and in what order, has no home.

## The core idea

The workflow is the main object you work with. Workflows are grouped by project folder in the eventual sidebar; the CLI must support the same project/workflow selection without a GUI. A workflow is a branching graph, not a linear prompt queue.

- Each node represents a phase's prompt/assignment, which can be refined through multiple turns rather than requiring a new node for each turn.
- A node is associated with an evolving native jj change. Its historical code revisions are native jj commits, not a separate Waypoints snapshot system.
- Users move backward and forward between nodes and workflows, inspect historical revisions, and explicitly continue work from a chosen revision.
- Connections express intended workflow relationships. Native jj ancestry is separate repository truth; show differences rather than silently forcing one to match the other.
- Upstream edits can make downstream work stale and introduce jj conflicts. Surface these consequences; do not automatically execute downstream nodes.
- Agent conversations stay in existing harnesses. A phase may use fresh scoped context or explicitly continue an upstream conversation; Waypoints does not archive transcripts.

## Project, workflow, and node example

```text
Project folder
└── Workflow
    └── Interfaces + placeholders
        ├── Authentication
        ├── Image uploading
        └── Settings
```

The three implementation nodes branch from the interfaces node's code. Each can evolve independently, and the user can return to the interfaces node or any implementation node to refine it. Combining sibling results is an explicit action, not an automatic consequence of sharing a parent.

Each node should let the user browse its native code revisions. Viewing an old revision is read-only; continuing from it is an explicit native jj operation whose exact behavior must be validated. Code history does not rewind conversation memory or automatically preserve the prompt text used at that time.

Use **workflow**, not a new application-level "session," for the sidebar item. Session IDs, when needed for integration, refer to harness-owned agent conversations.

## ComfyUI-style ideas worth borrowing

- Nodes are scoped assignments; connections describe selected upstream inputs and intended dependencies.
- Live streaming inside each node (watching output is a key value).
- Reuse existing results; when a node changes, show affected downstream work and let the user decide what to revisit.
- Gates: a node can pause for approval before passing output on.
- Fan-out and fan-in: try two approaches, then compare or merge.
- Graphs saved as plain files, so they can be diffed and shared.

## Example chain

Rough spec -> Plan -> (Approach A, Approach B in parallel) -> Review and compare -> Your approval gate.

## What is deliberately NOT being built

- An agent harness or agent loop. Existing agents do the agent work.
- A multibuffer or full diff editor. Zed and Delta already do this; hand off to them.
- A replacement for Delta. This sits above single threads and decides which ones exist.

## Is it actually useful? A test

Think of the last five times you chained prompts. If you ran roughly the same sequence more than once, copy-pasted output into the next prompt, or wished you could try two approaches at once, this earns its place. If every chain was different and exploratory, a plain chat thread is better.

## Naming

- "Node engineering" and "graph engineering" are vague (they sound like Node.js and graph databases).
- "Software factory" is a crowded term for autonomous, minimal-human pipelines. This tool is the opposite: the human steers each phase. Avoid it.
- Plain category wording: "plan-driven agent workflows" or "phased agent development".
- Product name candidates: Waypoint, Relay, Strata, Tack (a sailing term for changing course; pairs with Proa, which is also a boat). Check crates.io, GitHub, and domains before committing; short common words are often taken.
