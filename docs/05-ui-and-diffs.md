# UI and diff viewing

## Views

- **Plan blocks:** phases made of points, edited as a document.
- **Node canvas:** the phase graph with stale and conflict marks, version stacks per node. This is the hardest part (GPUI has no React Flow equivalent) and the least needed to prove the idea; build it last.
- **Phase view:** markdown notes and diffs for the selected phase.
- **Run stream:** live agent output for running phases (watching it is a core value).

## Seeing code introduced by each phase, without a multibuffer

1. **While a phase runs:** ACP edit tool calls carry diff content. Show a small card under the phase: file name, +N/-M, click to expand.
2. **After a phase:** ask jj for the diff of that change. Show the file list in the phase block; expanding a file shows hunks as read-only colored text.
3. **Deep review:** do not rebuild it. Offer "Open in Zed" on the phase's workspace, or an external diff tool via `jj diff --tool`. Check what the author uses for reviewing agent code today and target that first.

## Components

- Use `gpui-component` for Markdown rendering and read-only code or diff text (it has Markdown and simple HTML rendering, a code editor with Tree-sitter highlighting, dock layout, virtualized tables and lists, charts, and 60+ components).
- Avoid depending on Zed's internal editor crates; they are probably not meant to be embedded and may break between versions (not verified).
