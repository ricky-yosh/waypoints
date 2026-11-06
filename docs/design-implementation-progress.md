# Approved design implementation

## Reference inventory

| Reference file | Surface and state | Intended interaction | Coverage |
| --- | --- | --- | --- |
| Waypoints Dark Mode Workflow Overview.png | Upper frame at 100% with an anchored preview on a running node and a different idle node selected. Lower frame at 60% with compact cards and no preview. | Inspect a node, open its workbench, navigate dependencies and zoom. Keep permission, finalizing, failed run, and failed tool outcomes legible. | 100% and 60% captured and verified |
| Waypoints Dark Mode Assignment Tab.png | Floating workbench with an unsaved Markdown assignment and an edit/preview switch. | Edit, preview, save, resize, expand, close, and return. | Implemented and verified |
| Waypoints Dark Mode Output Tab.png | Floating workbench with running structured output, an expanded tool row, cancel, and a return-to-tail command. | Read and inspect activity while it streams, cancel a run, or jump to its latest output. | Implemented and verified |
| Waypoints Dark Mode Workflow Diff Tab.png | Floating workbench with a recorded revision, changed-file list, and unified diff. | Select a file and control split and wrapping while keeping the review target explicit. | Implemented and verified |
| Waypoints Dark Mode History Tab.png | Floating workbench with run and saved snapshot navigation and a failed run selected. | Inspect prior results and review changes when a saved revision is available. | Implemented and verified |

The overview shows the graph by itself. The four tab references share a floating workbench and a more compact graph presentation. The approved interaction contract keeps graph coordinates independent of overlay and output changes. Reference example text and activity are illustrative. Available capabilities and visible facts come from the application and backend contracts.

## Loop 1 · Graph and workbench shell

**Task and keyboard path.** Open a workflow, select a graph assignment with a click or Tab and Enter, then use the workbench without losing the dependency context. Use the workbench buttons by pointer or keyboard and Escape from within the workbench to close and return focus.

**Baseline.** The disposable fixture renderer captured all four tabs in both themes at 1200 × 860 logical pixels under `/tmp/waypoints-design-baseline/`. The assignment capture shows a full-width, flush split below the graph with a second layout activated by Hide graph. There is no close action for the workbench. The graph's scroll viewport changes size when the split changes. The normal sandbox prevented macOS pasteboard initialization. The fixture capture succeeded with desktop access.

**Acceptance.** Compare the floating frame and header against all four tab references and the unobstructed graph against the overview. Keep the graph mounted with stable camera and selection while switching, expanding, resizing, and closing the one workbench. Restore the prior floating size and keyboard focus. Preserve drafts and tabs across closing and reopening, and leave permission actions independent of workbench selection.

**Severity and confidence.** High, confirmed in source and baseline capture. The split changes the graph viewport and leaves no return-to-graph command. Implement the workbench at the workflow shell, with node state retained by its existing owner.

**Loop 1 evidence.** Replaced the split with one bottom-anchored nonmodal workbench. Expand and Restore share the same view, and closing keeps the selection and cached per-node workspace. The canvas owns a retained scroll handle per workflow. The preferences now save a floating height fraction. Existing split-height and collapse settings have no direct equivalent in the new layout.

The focused rendered test passed resize, stable graph geometry, expand/restore, draft retention, Escape focus restoration, and Enter to reopen. The native fixture harness passed floating/expanded interaction, assignment refresh, multiple permission dialogs, history and review freshness, and compact follow-up at 720 × 480 and 1.5× text. After captures for both themes and all tabs are under `/tmp/waypoints-design-shell/` and use the GPUI Fast headless renderer. The dark Assignment comparison shows the inset bottom frame, handle, and commands in the intended positions. Card density and connections still reflect the prior graph, and the title and tab content differ from the references.

## Loop 2 · Workflow overview and preview

**Task.** Read dependencies and monitored activity at a glance, zoom to see more of a workflow, and inspect a single assignment before opening its workbench. Keep selection and execution visually independent. Use pointer hover or Shift+Enter on a focused node for the preview, Escape to dismiss, and Enter on its command to open.

**Gap and acceptance.** The shell capture shows title-only cards and right-angle edges. The overview requires a dotted field, readable assignment and activity at 100%, and a compact 60% state that still names the run and important tool or snapshot exceptions. The preview must stay anchored, support pointer travel and keyboard activation, and leave card bounds and edges stable while activity changes. Severity medium for readability and high for truthful and keyboard-accessible run information. The screenshot and graph renderer confirm the geometry gap. The run panel owns the real bounded transcript and will supply the summaries.

**Loop 2 evidence.** Added a dotted field, curved dependencies, separate selection and run treatment, and 60–140% zoom. The anchored preview reads the saved prompt, bounded run-panel activity, pending tool permission, and run snapshot outcome. A tool failure and a run outcome remain separate. Run status is unknown until checked, and a missing bounded transcript has an explicit empty state. Cards keep fixed bounds as activity changes and scroll past the workbench through the same per-workflow camera.

The rendered tests passed pointer hover and activation, Shift+Enter and Escape focus return, two zoom sizes, and switching through the exposed graph while preserving the editor draft. The first 100% and 60% captures are under `/tmp/waypoints-design-overview/`. They confirmed progressive density and revealed a weak selection outline and an overview that sat too high at compact zoom. The next revision added a fixed-size selection outline and vertical centering for compact zoom.

## Loop 3 · Workbench tab content

| Task and reference | Baseline gap and user impact | Implementation and acceptance |
| --- | --- | --- |
| Edit and save instructions in Assignment | Source lacked line numbers and a distinct save area. Medium, confirmed in the baseline and source. | Use Kit's retained Editor, keep rendered Markdown preview and undoable external-save recovery, and align unsaved state and Save at the right. Verify typing, refresh and switching. |
| Inspect Output during a run | Executable setup occupied the run toolbar and status was on a separate row. Medium, confirmed in source and the baseline. | Show the known executable, run phase and cancel control together during execution. Keep structured tool disclosure, replay/gap notices and tail controls. |
| Inspect a file in Changes | View controls were far from the selected file and the target competed with them. Medium, confirmed in the baseline. | Group the target and change counts with file navigation. Keep the selected path, Unified, Split and Wrap together over Kit Diff. Check pinned targets, refresh and retained scroll. |
| Read a result in History | Runs and revisions used one full-width list with no selected detail. Medium, confirmed in the baseline. | Use a compact list beside the selected run or revision. Keep lifecycle, snapshot, native revision and review actions explicit. History does not contain an archived transcript or precomputed file deltas. |

The locked workspace suite passed with desktop access to the disposable test sockets. All 52 app tests passed in that run. Formatting, the locked app build and strict all-target, all-feature Clippy passed at the first Loop 3 checkpoint. Rendered captures of all four tabs and both overview states in light and dark are under `/tmp/waypoints-design-native.yGB34J/headless/`. The native fixture windows opened and reported valid window IDs. macOS `screencapture` returned `could not create image from window` for each ID. Computer Use subsequently captured the isolated native windows successfully.

## Final comparison and validation

| Reference | Observed comparison |
| --- | --- |
| Workflow Overview | The native 100% preview now opens beside the node within the canvas, leaving the header, selected idle node and rightmost failure legible. At 60%, the native graph fits horizontally and the permission, finalization, run failure and tool failure labels fit their cards. The compact view drops assignment descriptions and routine output. The panel remains anchored when its text changes. |
| Assignment | The native capture shows the inset frame, Edit and Preview, save status, and highlighted source with line numbers. The final paired fixture renders also show the footer for an eligible saved session. The compact window uses the existing follow-up dialog. |
| Output | The native capture shows the running toolbar, cancel command, bounded transcript and expanded tool data. The transcript retains its reading position while output arrives and offers Jump to latest after scrolling away from the tail. |
| Changes | The native capture shows target metadata in the file column and Unified, Split and Wrap beside the selected path. The stale-current warning remains prominent. The rendered regression checks retain pinned targets and diff position across permission dialogs. |
| History | The native capture shows the selected run beside the run and revision list. The final minimum-window harness selected a saved run, found its review command, then changed selection and verified the command changed with it. |

**Backend contract.** Agent identity and configuration use the existing ACP executable path. Tool details use the ACP event payload and preserve replay, gap and truncation context. History shows recorded metadata and revision actions, with a recent output section scoped to the matching retained run. Older runs show an unavailable-output message. File deltas come from the explicit review target. The fixtures represent these states without requiring agent credentials.

**Native evidence.** Dark native window captures are in the chat tool results. The final GPUI Fast fixture renders in both themes are under `/tmp/waypoints-design-verified.0hDCZh/`. The same directory contains the final native QA log. The 720 × 480 harness with 1.5× text passed resize, stable graph bounds, Escape and keyboard preview, zoom reset, draft and external-save handling, Assignment and Output follow-up, multiple permission dialogs, History selection and freshness, review controls, and closing during a running state. The rendered tests cover pointer preview, switching nodes through the exposed graph, scrolling, retained draft and review state, and streamed output anchoring.

**Completion.** The reference comparisons and isolated native interactions are verified. Formatting, the locked app build, strict all-target and all-feature Clippy, and the final locked workspace suite passed. The workspace suite reported 273 passing tests, including 52 in the app. An earlier rerun hit two intermittent backend restore assertions. Both passed in the final full run with four test threads. The final workspace test log is at `/tmp/waypoints-design-verified.0hDCZh/workspace-tests.log`.
