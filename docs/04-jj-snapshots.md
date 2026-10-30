# jj snapshots per phase

## Model

- Each phase node is associated with an evolving jj change, initially built on its selected upstream code revision. Saved workflow connections and native ancestry are distinct relationships.
- Parallel phases are sibling changes on the same parent.
- Simultaneous editing of different nodes needs separate working directories: allocate jj workspaces on demand. Idle nodes and browsing historical revisions do not each require a permanent workspace. Two agents editing different code states must not share one folder.
- Combining parallel results is explicit: for a native merge, create a change with the selected results as parents. jj stores conflicts inside the commit instead of blocking, so a conflicted merge is a state clients can show.
- Editing an earlier phase: jj automatically rebases everything above it. The app marks those phases stale; conflicted ones get a conflict badge.
- A change ID normally follows evolving work across rewrites; a commit ID identifies one exact revision. Divergence can give a change ID multiple visible revisions and requires an explicit choice.
- jj's operation log supports repository recovery, not undo of workflow metadata, conversations, or external side effects. Workflow edits have their own transactional undo history.

## Browsing and continuing node history

A node can expose multiple native code revisions without creating a workflow node for each revision. Viewing an older revision is read-only. Continuing work from it requires an explicit jj operation; verify restore/branch commands and revision retention before specifying exact behavior or guaranteeing indefinite history.

Native code revisions do not preserve historical prompt text or rewind agent memory. Workflow prompt edits are stored separately; matching a prompt version to a code revision requires an explicit association. If an associated native change disappears, retain the saved node with a missing-change indication rather than deleting the workflow or automatically resurrecting repository history.

## Two kinds of conflict

1. **Code conflicts:** jj handles these.
2. **Context staleness:** downstream reasoning or work was built on old upstream output even if the code merges cleanly. jj cannot see this; the app tracks affected workflow dependencies. When an upstream phase changes, mark affected work stale and let the user choose whether to rerun, create an alternative, or keep the result. Connections do not automatically trigger execution.

## Integration approach

Start by shelling out to the `jj` CLI. A Rust library for jj exists, but use it only if the CLI becomes limiting. The agent never needs to know jj exists; the app snapshots after each phase.

## Commands to confirm in a scratch repo (from memory, verify)

- Create a change per phase; show a phase's diff with `jj diff -r <change>` (`--git`, `--stat`, `--summary`).
- Add workspaces for parallel phases; merge with a multi-parent change.
- Edit an early change and observe the rebase and conflicts (`conflicts()` revset).
- Whether Zed shows a phase's diff in a colocated repo (git HEAD pointing at the working copy's parent).
