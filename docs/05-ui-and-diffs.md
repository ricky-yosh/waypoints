# UI and diff viewing

This is a future UI sketch, not an implemented screen inventory or an ordered
roadmap. The app has a readonly multi-project browser with a resizable, hideable
project/workflow navigator and remembered workspace layout. It does not yet edit
workflows or display a graph canvas. The GUI must use the shared backend;
see [current scope](20-core-backend-status.md) and the
[native-feedback decision](02-architecture.md#decision-reflect-native-jj-changes-not-inferred-context-staleness).

## Views

- **Plan blocks:** phases made of points, edited as a document.
- **Node canvas:** the workflow graph reflecting native revisions and conflict flags, with revision history inside each node rather than extra revision nodes. No inferred-context-staleness marks or gates for v1.
- **Phase view:** markdown notes and diffs for the selected phase.
- **Run stream:** live agent output for running phases (watching it is a core value).

## Seeing code introduced by each phase, without a multibuffer

1. **While a phase runs:** ACP edit tool calls can carry proposed diff content, but shell edits may have none. Distinguish proposals/replayed events from completed work and show stream gaps; do not treat events as authoritative file state.
2. **After a phase:** use backend review of its exact saved revision, or explicitly select the current recorded revision. These may differ. Native jj supplies the summary/patch; the GUI must not snapshot or independently invoke jj for review.
3. **Deep review:** editor handoff remains to be designed and validated. Zed or a native external diff tool is a candidate, not an implemented command or fixed requirement. Do not build a custom multibuffer.

## Components

- Use the selected `gpui-kit` dependency and its component layer. Consult the installed design/coding guides and verify APIs against the pinned version before choosing Markdown or read-only diff components.
- Avoid depending on Zed's internal editor crates; they are probably not meant to be embedded and may break between versions (not verified).
