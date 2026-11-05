# Working in the desktop app

Open a project and select a workflow in the sidebar. The graph shows saved
dependencies, with arrows pointing downstream. Click a node or focus it and
press Enter to open its workspace below the graph. Scroll the graph to reach
more nodes, drag its bottom divider to resize it, or use **Hide graph** to give
the workspace more room. Waypoints remembers the divider and visibility.

- **Assignment** edits the saved prompt. Use **Edit** for Markdown source and
  **Preview** for selectable rendered Markdown, including code blocks with copy
  actions. Saving updates the instructions used for future fresh runs; an
  active run keeps its captured instructions. Drafts, preview mode, and editing
  position remain in the current app session when you switch nodes. Unsaved
  status stays visible in both modes.
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
  latest rows and stops following when you scroll upward; **Latest output**
  returns to the newest content. The run outcome and snapshot outcome are
  separate.
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
- **History** separates run records from native revisions. Runs show lifecycle
  state, readable timestamps, continued-from references, and snapshot state as
  distinct facts. Native revisions show a short operation description, current
  head and conflict indicators, associated run references, and a short revision
  ID with the full value available through copy. **Load older revisions** uses
  the backend's opaque pinned-operation cursor; already-loaded rows stay in
  place if a later page fails. A run can open a review when it has a saved
  snapshot. Refresh to fetch newer history.

A pending permission appears in the workflow header for a monitored run. Open
its action to inspect the tool request over the current workspace. Choose one
of the backend's single-use options or dismiss the dialog to leave the request
pending. The review stays in place. After a reply or expiry, the dialog shows
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

The four node surfaces can be rendered without a window server using the
fixture-backed screenshot binary:

```sh
cargo run -p waypoints-app --features screenshot-fixtures \
  --bin waypoints-screenshots -- .screenshots
```

It opens the real workspace views through GPUI Fast's headless renderer at
1200×860 in light and dark themes and writes `assignment-preview-*.png`,
`output-*.png`, `changes-*.png`, and `history-*.png`. The data is deterministic
fixture data; this exercises layout, typography, syntax highlighting, and theme
contrast, not a real agent session or backend round trip.
