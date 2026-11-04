# Working in the desktop app

Open a project and select a workflow in the sidebar. The graph shows saved
dependencies, with arrows pointing downstream. Click a node or focus it and
press Enter to open its workspace below the graph. Scroll the graph to reach
more nodes, drag its bottom divider to resize it, or use **Hide graph** to give
the workspace more room. Waypoints remembers the divider and visibility.

- **Assignment** edits the saved prompt. Saving updates the instructions used
  for future fresh runs. An active run keeps its captured instructions. Drafts
  remain in the current app session when you switch nodes.
- **Output** starts a fresh run using the saved assignment and an absolute path
  to an installed ACP executable. The backend requires an associated native
  change and a verified, ready workspace. Use the [CLI](11-backend-cli.md) to
  create these. To reuse an eligible session, enter separate follow-up
  instructions and choose **Continue session**. Output is bounded live activity,
  and the backend can expire it. The run outcome and snapshot outcome are separate.
- **Changes** shows the current recorded revision or an exact revision selected
  from History. Pick a changed file above the patch. Returning to a node restores
  its tab, review target, file, and diff position within the session.
- **History** separates run records from recent native revisions. A run can open
  a review when it has a saved snapshot. Refresh to fetch newer history.

A pending permission appears in the workflow header for a monitored run. Open
its action to inspect the tool request over the current workspace. Choose one
of the backend's single-use options or dismiss the dialog to leave the request
pending. The review stays in place. After a reply or expiry, the dialog shows
that the request is no longer pending.

Live ACP tool updates may describe proposed edits. For completed code review,
Waypoints requests the exact native patch from the backend and displays it in
Kit's read-only diff viewer. The desktop app does not open the store, run jj for
review, or restore code from history.
