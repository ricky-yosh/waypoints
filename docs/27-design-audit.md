# Design Guides audit

Audit of the native Waypoints desktop app against
`.agents/skills/gpui-kit-design-guides/references/design-guides.md`.

Scope: every application render path in `crates/waypoints-app/src` — shell,
sidebar, graph, workbench, assignment, output, changes, history, dialogs, and
settings. Five parallel read-only audit passes covered the shell, workbench,
run panel, history/graph, and app chrome/settings; every reported finding was
re-checked against the source before fixing. The audit also used light/dark
headless captures, the app test suite, and the native QA harness at the minimum
window size and 1.5× text.

## Findings and fixes

| Area | Finding | Guide rule | Resolution |
| --- | --- | --- | --- |
| Graph status | A running run used the success color, making progress look completed. | Color and themes: semantic colors keep their meaning. | Running now uses the informational role (`node_workspace.rs`, `run_state_visual`). |
| Graph status | Phase pills put a saturated status color on 12 px text over a light tint, which is hard to read in the light theme. | Accessibility: text contrast; status not by color alone. | Pills keep the normal foreground and carry the hue in a small dot (`workflow_graph.rs`). |
| Graph selection | The selected node outline used the informational color. | Interaction states: selection uses selection emphasis, not a status role. | Selected nodes now use the primary role (`workflow_graph.rs`). |
| History metadata | “Current head” used the link color even though it is not interactive. | Color and themes: link styling is for links, not decoration. | Uses the primary role (`node_history.rs`). |
| History rows | Narrow history rows truncate the timestamp/commit subtitle with no recovery path. | Accessibility and truncation: truncated copy needs a recovery path. | The full subtitle is exposed in the row tooltip (`node_history.rs`). |
| History states | Empty runs/revisions lists only stated that they were empty. | Designing data-heavy interfaces: a useful empty state explains the next action. | Copy now explains where runs/revisions come from (`node_history.rs`). |
| History virtualization | The initial history row estimate was a fixed 400 px, so zoom did not influence the first measurement. | Zoom: virtualized row measurement must follow the rem scale. | The estimate now uses `window.rem_size() * 25` (`node_workspace.rs`). |
| Errors | Save, review, history, and older-page failures showed raw backend strings as ordinary body text. | Interaction states: errors state what happened and how to recover; error state must be visible. | Messages now name the failed action and carry a danger marker (`node_workspace.rs`, `node_history.rs`). |
| Run errors | Run and permission errors were indistinguishable from nearby status text. | Interaction states: error state must be distinct. | Both now use the shared danger status treatment (`node_runs.rs`). |
| Forms | The inline and dialog follow-up composers relied on a placeholder and accessible name instead of a visible label. | Forms: every field has a visible label. | Persistent “Follow-up instructions” labels were added; the placeholder now gives an example (`node_runs.rs`). |
| Forms | The agent executable field had no visible label. | Forms: every field has a visible label. | Inline “Agent executable” label added; accessible name aligned (`node_runs.rs`). |
| Empty state | An empty workspace said “No projects open” but offered no action in the work area. | Start from the task: the next action must be clear; empty states explain the next action. | The detail pane now offers “Add project…” (`workspace.rs`, covered by `empty_workspace_offers_a_visible_add_project_action`). |
| Sidebar hierarchy | “Projects” was styled like another row. | Hierarchy: section titles separate meaningful regions. | Small, semibold, muted section label (`workspace.rs`). |
| Segmented controls | Edit/Preview and Unified/Split were independent pills rather than one silhouette. | Radius and interaction states: a segmented control is one silhouette with a single divider and a persistent selected fill. | Both are `ButtonGroup`s inside one muted container; selected segments carry the stronger fill (`node_workspace.rs`). |
| Overlay boundary | The floating workbench used the focus-ring token for its static boundary. | Color and themes: ring tokens belong to interaction/focus. | Boundary uses `border` (`workspace.rs`). |
| Notices | The workspace-preferences warning could not be dismissed. | Feedback and overlays: topmost dismissible surface and focus return. | Added a close action that clears the notice and returns focus to the sidebar toggle (`workspace.rs`). |
| Menus | “Remove from Sidebar” sat directly against Refresh/Retry. | Components: separate different scopes in menus. | Added a menu separator (`workspace.rs`). |
| Menus | “Show/Hide Projects” did not name a state or a persistent command. | Interface language: name the object and the verb. | Renamed to “Show Projects Sidebar” and checked while visible (`workspace.rs`). |
| Copy | “Try again” and “Retry” described the same command differently. | Interface language: one product lexicon. | Standardized on “Retry” (`node_workspace.rs`). |
| Copy | “Add Project…” in the title-bar tooltip disagreed with the sentence-case action elsewhere. | Capitalization: sentence case for English UI. | Tooltip and accessible name now read “Add project…” (`workspace.rs`). |
| Identity | Every markdown fenced block reused the `copy-code` action id, so hover/focus/accessibility state collided among blocks. | Stable identity: repeated elements need unique ids; interaction state must be reliable. | Copy actions derive an id from the owning view and the block's source span (`node_workspace.rs`). |
| Copy | Folder-picker failure reported the error without a recovery step. | Interface language: errors include recovery when useful. | Appends “Try again.” (`workspace.rs`). |
| Permissions | The run panel said “Permission needed” but the only answer path sat in the graph header. | Start from the task: actions that must remain immediately available. | The running toolbar now has a labeled “Review permission…” action (`node_runs.rs`). |
| Permissions | The dialog title was run metadata, the body was raw JSON, and the decision scope was unstated. | Confirmation dialogs: title names the decision; body adds scope and consequence. | Title names the requested tool and node; the body states the run is paused and that the answer applies only to this request; JSON is a copyable code block (`node_runs.rs`). |
| Permissions | The permission trigger label was only “Node #N” and omitted the dialog ellipsis. | Button labels name the action and object; dialog-opening commands take “…” . | “Review permission · Node #N…” with a full accessible label (`node_runs.rs`). |
| Permissions | “Dismiss” left the request pending under a different verb. | Use `Cancel` consistently for leaving without committing. | The bottom action is `Cancel` and is disabled while the answer is in flight (`node_runs.rs`). |
| Dialogs | The follow-up dialog opened with focus on its container, not its only field. | Focus order follows visual and task order; keep focus in a logical place. | Focus moves to the follow-up textarea after the dialog opens (`node_runs.rs`). |
| Review | A stale “Review current” disabled the very action that would recover it. | Error state: state what happened and how to recover. | The notice carries a “Review latest” action; the header action stays enabled while stale (`node_workspace.rs`). |
| Review | Header chrome used three equal outline buttons. | Emphasis budget; ghost for quiet toolbar actions. | Expand/Restore and Close are ghost; Continue session remains outline (`node_workspace.rs`). |
| Review | File rows exposed only the path to assistive technology; status and selection were visual. | Accessibility: status not by color alone; selected state must be announced. | Accessible labels include added/deleted/modified; buttons keep selected semantics (`node_workspace.rs`). |
| Review | The diff path heading was smaller than the file rows and the directory line ignored the status lane. | Hierarchy and alignment spines. | The path is `text_sm`/medium, counts use tabular mono figures, and directory lines align under the filename (`node_workspace.rs`). |
| Review | “Saved assignment” was a noun, not a state. | Name states with adjectives or short phrases. | The clean state reads “Saved” (`node_workspace.rs`). |
| Tabs | The workbench tabs were ordinary buttons to assistive technology. | Standard component for the semantic role; selected state must be visible. | The workbench uses `TabBar` with standard tab keyboard behavior and persistent selection (`node_workspace.rs`). |
| Workspace focus | Hiding the sidebar or removing a project moved focus to an unstyled window root. | Focus remains visible and is restored after actions. | Focus returns to the visible sidebar toggle (or Add project after the picker alert) (`workspace.rs`). |
| Workspace scrolling | Sidebar and detail scrollbars were inset by the surface padding. | A scrollbar belongs at its scrolling region's trailing edge. | Padding moved inside scroll content; scrollbars sit on the split and panel edges (`workspace.rs`). |
| Workspace proportion | The sidebar could consume most of the width at the minimum window size. | Navigation stays subordinate; detail gets the surplus. | Sidebar max is capped at 40% of the viewport as well as the absolute token (`workspace.rs`). |
| Workspace errors | Project-load diagnostics were available only in a hover tooltip. | Error state must show what happened and how to recover. | The diagnostic is inline under the status with Retry (`workspace.rs`). |
| Workspace empty state | “No active workflows” offered no next step. | Useful empty state explains the next action. | A visible Refresh action sits beside the message (`workspace.rs`). |
| Menus | No Edit menu and no keyboard path to close a window. | Desktop conventions: menus, keyboard access, platform shortcuts. | Added Edit (Undo/Redo/Cut/Copy/Paste/Select All), File > Close Window, `Cmd/Ctrl-W`, and a state-aware View menu (`lib.rs`, `workspace.rs`). |
| Menus | Refresh was always enabled and the sidebar item was a slash label. | Hide or disable inapplicable actions; name the object and state. | Menus are rebuilt on selection/sidebar changes: “Show Projects Sidebar” is checked and “Refresh Project” is disabled without a selection (`workspace.rs`). |
| Graph | The canvas had no scrollbar even when the graph overflowed. | Scrollbar belongs to the scrolling region; region-owned scrolling. | Both scrollbars are attached to the graph canvas (`workflow_graph.rs`). |
| Graph | A hover preview had no visible close action. | Escape dismisses the topmost layer; pointer users need a dismissal path. | Added an accessible close button; Escape still returns focus (`workflow_graph.rs`). |
| Graph | The zoom reset label was the current value, not the action. | Labels name the action, not the gesture or component. | “Reset zoom · 100%” names the action and keeps the current value (`workflow_graph.rs`). |
| Graph | A failed snapshot read as ordinary muted metadata. | Error state must be distinct and consistent. | Failure uses the danger role while the text still states what failed (`workflow_graph.rs`). |
| Graph | The Shift+Enter preview shortcut was undiscoverable. | Expose shortcuts in menus or tooltips. | The preview footer names the shortcut (`workflow_graph.rs`). |
| Settings | The utility window clipped at its own minimum and at larger text. | Responsive desktop windows: scroll only the overflowing region. | The content scrolls, and the default/minimum heights grew (`settings.rs`). |
| Settings | “Appearance” was plain text and the radio group had no accessible name. | Hierarchy and accessible names. | The heading has a heading role/level and each radio is labeled “Appearance: …” (`settings.rs`). |
| Settings | Title, description, and control group shared one gap. | Vertical rhythm shows grouping. | Heading/description use a tight column; the control group is separated by a larger gap (`settings.rs`). |

## Checked and clear

- No raw hex, `rgb`, or `hsla` values in application UI; colors come from
  `cx.theme()`.
- Remaining `px(...)` uses are physical window bounds, divider hit margins,
  persisted region sizes, raster hairlines, or test geometry.
- Button variants are limited to three real commits (Save, Start run, the
  follow-up dialog commit); no primary button is used for ordinary toolbar
  actions.
- No `Link` is used for in-app commands.
- Icon-only actions have accessible names, and ambiguous ones have tooltips.
- Selected navigation rows, file rows, tabs, and segments use their own
  surface rather than a leading-edge bar.
- Overlays dismiss with Escape and restore focus (workbench, project menu,
  permission dialog, graph preview).
- Empty, loading, error, disabled, and in-progress states are represented in
  the shell, graph, workbench, runs, and history.
- Long lists use the retained list/scroller components; scrollbars belong to
  their scrolling regions.
- No ambient animation was found; the hover preview keeps one surface while
  moving between nodes and only dismisses after the pointer leaves.
- Light and dark captures were reviewed for contrast and surface hierarchy.

## Verification

```
cargo fmt --all -- --check                      # clean
cargo test -p waypoints-app --all-features      # 53 passed; 0 failed
cargo clippy -p waypoints-app --all-targets --all-features -- -D warnings
cargo run -p waypoints-app --features native-qa --bin waypoints-native-qa
cargo run -p waypoints-app --features screenshot-fixtures --bin waypoints-screenshots -- .screenshots
```

Native QA passed at 720×480 and 1.5× text for the floating workbench, Escape
and keyboard reopen, assignment refresh, permissions, history freshness,
review focus restoration, follow-up layout, and running-toolbar behavior.
All twelve light/dark captures were regenerated and checked for the review,
history, assignment, output, and compact graph surfaces. The history split was
verified as a resizable master–detail pair, and the review header was checked
after the stale-notice and segmented-control changes.
