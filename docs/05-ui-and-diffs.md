# Working in the desktop app

Open a project and select a workflow in the sidebar. The graph shows saved
dependencies, with arrows pointing downstream. Click a node or focus it and
press Enter to open its floating workbench. Scroll the visible graph to reach
more nodes, or drag the workbench's top edge to resize it. **Expand** fills the
workflow area and **Restore** returns to the previous floating size.
**Close** or Escape from the workbench returns focus to the graph node.
Closing preserves per-node context and lets runs continue. Hover over a node
or press Shift-Enter on a focused node to preview its assignment and recent
monitored activity. The zoom controls progressively reduce card detail.
A node whose run status has yet to be checked shows **Unknown**.

- **Assignment** edits the saved prompt. Use **Edit** for Markdown source and
  **Preview** for selectable rendered Markdown, including code blocks with copy
  actions. Saving updates the instructions used for future fresh runs; an
  active run keeps its captured instructions. Drafts, preview mode, and editing
  position remain in the current app session when you switch nodes. Unsaved
  status stays visible in both modes. A project refresh also reconciles the
  selected assignment: a clean draft follows the newly saved text, while an
  unsaved draft is kept with a warning and an undoable **Load saved assignment**
  action. When the node has an eligible saved session, the footer accepts
  follow-up instructions and opens Output when you continue.
- **Output** starts a fresh run using the saved assignment and an absolute path
  to an installed ACP executable. The backend requires an associated native
  change and a verified, ready workspace. Use the [CLI](11-backend-cli.md) to
  create these. To reuse an eligible session, enter separate follow-up
  instructions and choose **Continue session**.

  Output is a bounded, structured transcript rather than one flattened string.
  Assistant prose renders as selectable Markdown; reasoning is shown separately;
  tool rows keep a stable identity while their status and summary update in
  place; replay boundaries, expired-event notices, and truncation notices stay
  outside the Markdown. Expand a tool row to inspect its original input/output
  as JSON with a copy action. Output follows the tail when you are reading the
  latest rows and stops following when you scroll upward; **Jump to latest**
  returns to the newest content. At the minimum window height, expand the workbench for more room. When the inline
  composer does not fit, **Continue session…** moves beside the node title and
  opens a follow-up dialog. The run outcome and snapshot outcome are separate.
- **Changes** shows the current recorded revision, a saved run snapshot, or an
  exact native revision. The target label is always explicit, and the commit ID
  is secondary monospace metadata with a copy action. The changed-file sidebar
  lists status, additions/deletions, directories, renames, and binary files
  compactly for long paths and many files. Choose **Wrapping lines** or
  **Unwrapped lines** and switch between unified and split views; unwrapped is
  the readable default. Syntax highlighting is provided by syntect. The diff
  viewport owns its scrolling, and returning to a node restores its tab, review
  target, selected file, and per-file diff position within the session.
  Current review is marked stale when a run records a new revision and only
  reloads when you ask. Historical and saved-run targets remain pinned.
- **History** separates run records from native revisions. Select an entry in
  the list to inspect its details alongside the list. Runs show lifecycle
  state, readable timestamps, continued-from references, and snapshot state as
  distinct facts. Native revisions show a short operation description, current
  head and conflict indicators, associated run references, and a short revision
  ID with the full value available through copy. **Load older revisions** uses
  the backend's opaque pinned-operation cursor; already-loaded rows stay in
  place if a later page fails. A run can open a review when it has a saved
  snapshot. If a run records new history while the tab is open, the toolbar
  announces it; Refresh fetches the newer runs and revisions.

  **Refresh limitation:** unlike a failed older-page request, a full History
  refresh replaces the loaded view with a loading state. If its run-list request
  fails, the tab shows an error instead of retaining the previously loaded rows.
  Retry the refresh when the backend is available.

A pending permission appears in the workflow header for a monitored run. Open
its action to inspect the tool request over the current workspace. Choose one
of the backend's single-use options or dismiss the dialog to leave the request
pending. When several runs need permission, each request appears as a compact
**Node #** action that wraps inside the workflow header, so no request is hidden
offscreen. The review stays in place. After a reply or expiry, the dialog shows
that the request is no longer pending.

Live ACP tool updates may describe proposed edits. For completed code review,
Waypoints requests the exact native patch from the backend and displays it in
Kit's read-only diff viewer. The desktop app does not open the store, run jj for
review, or restore code from history.

Empty patches, failed review loads, renames, binary files, conflicts, and
unknown file languages are shown honestly; the app does not invent a patch or
guess a language. See the [headless capture harness](#validation) used to
inspect the four node surfaces without a window server.

## Icons

Controls use the embedded Lucide SVG catalog through GPUI Kit. Tabs combine an
icon with a label; secondary utilities such as reload, wrap, split, and refresh
are icon-only with tooltips and accessible labels; primary actions keep their
labels. The app currently embeds the complete catalog for iteration speed; see
[technical debt](26-tech-debt.md#icon-assets) for the planned selective bundle.

## Validation

The native fixture QA binary opens a real window at the minimum size with 1.5×
text and drives the assignment refresh, multiple permission requests, history
freshness, review controls, and compact follow-up dialog. It uses fixture data,
not a backend or agent session:

```sh
cargo run -p waypoints-app --features native-qa --bin waypoints-native-qa
```

A passing native fixture run prints each completed step and a final standalone
`NATIVE_QA_OK`. This harness uses deterministic run and review fixtures. When launching the normal app for manual QA,
point `WAYPOINTS_WORKSPACE_FILE` at an isolated absolute preferences file and
`XDG_STATE_HOME` at a disposable state directory; macOS otherwise uses
`~/Library/Application Support` for preferences.

The fixture renderer captures the graph overview and four workbench tabs.

```sh
cargo run -p waypoints-app --features screenshot-fixtures \
  --bin waypoints-screenshots -- .screenshots
```

It captures the overview at 1440 × 510 and the workbench at 1280 × 720 in
light and dark themes. The filenames use the surface names
`overview-preview`, `overview-compact`, `assignment-preview`, `output`,
`changes`, and `history`. Pass `--inspect <surface> <light|dark>` in place of
the directory to open a fixture in a native window. Set `XDG_STATE_HOME` to a
disposable directory before trying actions that contact the backend.
