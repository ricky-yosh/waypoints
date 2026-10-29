# jj snapshots per phase

## Model

- Each phase is one jj change, built on its parent phase's change.
- Parallel phases are sibling changes on the same parent.
- Parallel work needs separate working directories: use `jj workspace` per phase. Two agents must not share one folder.
- Merging parallel results: a change with both as parents. jj stores conflicts inside the commit instead of blocking, so a conflicted merge is a state the UI can show.
- Editing an earlier phase: jj automatically rebases everything above it. The app marks those phases stale; conflicted ones get a conflict badge.
- jj change IDs are stable across rewrites, so a phase block can keep pointing at its snapshot after edits.
- jj's operation log gives undo for free.

## Two kinds of conflict

1. **Code conflicts:** jj handles these.
2. **Context staleness:** the chat behind a downstream phase was built on old upstream output even if the code merges cleanly. jj cannot see this; the app tracks it (like ComfyUI marking downstream nodes dirty). When an upstream phase changes the user chooses: re-run from there, fork a new chain from the edited seed, or keep the old result. Default behavior (auto re-run vs mark stale and ask) is an open question.

## Integration approach

Start by shelling out to the `jj` CLI. A Rust library for jj exists, but use it only if the CLI becomes limiting. The agent never needs to know jj exists; the app snapshots after each phase.

## Commands to confirm in a scratch repo (from memory, verify)

- Create a change per phase; show a phase's diff with `jj diff -r <change>` (`--git`, `--stat`, `--summary`).
- Add workspaces for parallel phases; merge with a multi-parent change.
- Edit an early change and observe the rebase and conflicts (`conflicts()` revset).
- Whether Zed shows a phase's diff in a colocated repo (git HEAD pointing at the working copy's parent).
