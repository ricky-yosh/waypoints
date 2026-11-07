# Working in the desktop app

The canonical desktop frontend is the Rune canvas in `crates/waypoints-app`.
Open a project and select a workflow in the Projects panel. The graph shows
saved dependencies, with arrows pointing downstream. Click a card to select it;
double-click it or press Return while focused to open the floating workbench.
Drag the canvas to pan, pinch or ⌘-scroll to zoom, and drag a card to change its
presentation position without editing dependencies. **Expand** fills the
workflow area and **Restore** returns to the floating size. Closing the
workbench with Escape or Close keeps the document selection and returns focus
to the graph card. Runs continue when the workbench closes.

Hover a card for 260 ms or press Shift+Return on a focused card to preview its
assignment and status. A node whose run status has not been checked shows
**Unknown**.

- **Assignment** edits the saved prompt. Use **Edit** for Markdown source and
  **Preview** for selectable rendered Markdown, including code blocks. Saving
  updates the instructions used for future fresh runs; an active run keeps its
  captured instructions. Per-node drafts, preview mode, editing position, tab,
  review target, and diff position remain in the current app session when you
  switch nodes. Unsaved status stays visible in both modes. A backend refresh
  reconciles the selected assignment: a clean draft follows the newly saved
  text, while an unsaved draft is kept with a warning and a **Load saved**
  action. In compact workbenches the editor and preview scroll above the footer;
  Expand gives them more room. When the node has an eligible saved session, the
  footer accepts follow-up instructions and opens Output when you continue.
- **Output** starts a fresh run using the saved assignment and an absolute path
  to an installed ACP executable. The backend requires an associated native
  change and a verified, ready workspace. Use the [CLI](11-backend-cli.md) to
  create these. To reuse an eligible session, enter separate follow-up
  instructions and choose **Continue session**.

  Output is a bounded, structured transcript. Assistant prose renders as
  selectable Markdown; reasoning is shown separately; tool rows keep a stable
  identity while their status and summary update; replay boundaries, expired
  events, and truncation notices stay outside the Markdown. Expand a tool row to
  inspect its original input/output as JSON. Output follows the tail while you
  are at the bottom and offers **Jump to latest** after scrolling away. Run
  outcome and snapshot outcome are separate facts. Pending permissions appear
  as single-use action cards; a reply is consumed once, and an expired request
  reports the backend error.
- **Changes** shows the current recorded revision, a saved run snapshot, or an
  exact native revision. The target label is explicit, and the commit ID is
  secondary monospace metadata with a copy action. The changed-file pane starts
  at 190 points and has a draggable divider (140–360 points); the diff takes the
  remaining width. Choose wrapping or unwrapping and switch unified/split views.
  Line selection and copy are available. A current review is marked stale when
  a run records a newer revision and reloads only on request; run and revision
  targets stay pinned. File status, renames, binary files, conflicts, empty
  patches, and errors are shown honestly. Syntax rendering uses Rune Kit's
  fallback.
- **History** separates run records from native revisions. Select an entry to
  inspect details beside the list. Runs show lifecycle state, timestamps,
  continued-from references, and snapshot state. Native revisions show the
  native operation description, current-head/conflict indicators, associated
  runs, and a short revision ID. **Load older** uses the backend's opaque pinned
  cursor; loaded rows stay if a later page fails. Saved runs and revisions hand
  off to Changes. If newer history is recorded, the toolbar announces it and
  **Load newest** refreshes.

  **Refresh limitation:** a full History refresh replaces the loaded view with a
  loading state. If its request fails, the tab shows an error instead of
  retaining the previous rows. Retry when the backend is available.

Closing the workbench does not cancel its run. The app never opens the store,
runs jj for review, or restores code from history; those remain backend/CLI
operations.

## Icons and chrome

Rune Kit's `IconButton` plus custom SVG `Icon`s provide the HUD controls (zoom,
reset, project sidebar, settings). Every icon button keeps a tooltip and
accessible label. The HUD starts below the macOS traffic lights while keeping
the normal left margin; the project panel starts below the canvas hint. macOS
native menus are installed by the app; Linux/Windows use the in-app controls.

## Validation

The repeatable checks and headless scenes are documented in
[Rune validation](port-info/validation.md). Native menus, IME, VoiceOver,
close/reopen, and multi-display behavior require a person; the current evidence
and limitations are in [cutover readiness](port-info/readiness.md).
