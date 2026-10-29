# Concept

## The problem

Work with coding agents often means chaining prompts by hand: run one, wait and watch the output (watching is useful), copy results into the next prompt, and lose track of which results still hold when something upstream changes. Delta (from the Zed team) makes single threads feel fluid, but the plan that decides which threads exist, and in what order, has no home.

## The core idea

The plan is the main object you work with.

- A plan is a document of phases; each phase has points (Step 1: pt 1, 2, 3; Step 2: pt 1, 2).
- Running a phase gives it an agent session. That session is either a fork of an earlier phase's chat, or a fresh one seeded with the plan text, so context is preserved.
- Finishing a phase stores its result as a snapshot (a jj change), so each phase is preserved and the original "seed" can be revisited and iterated.
- Editing an earlier phase marks everything downstream as stale (and shows conflicts if jj reports them).

## ComfyUI-style ideas worth borrowing

- Nodes are steps; edges pass one step's output into the next prompt.
- Live streaming inside each node (watching output is a key value).
- Caching: change one node and only it and its dependents re-run.
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
