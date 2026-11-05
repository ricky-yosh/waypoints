# Goal: Polish the Waypoints node workspace into a coherent native work environment

Improve the existing Waypoints node workspace so that reading assignments, following agent work, reviewing code, and understanding history feel deliberate and polished—closer in usability to Zed or Delta, without copying their entire interface or introducing a heavyweight editor framework.
Implement the improvements, inspect them in the running native application using computer use, and iterate until the workflows below work well. Do not stop at a plausible screenshot or a passing build.
Preserve the current structure:
- Project/workflow navigation in the sidebar.
- Compact horizontal dependency graph above the node workspace.
- Resizable and collapsible graph region.
- Full-width node workspace with Assignment / Output / Changes / History.
- Per-node draft, tab, selected review target/file, and scroll retention.
- Permission requests that never steal navigation.
This is a focused polish and correctness pass, not a new layout redesign.
Read the repository instructions and load the GPUI Kit Design Guides and Coding Guides before making UI or architecture decisions. Use the actual locked component source to verify APIs. Public docs may describe newer APIs.
Use GPUI Kit components before inventing primitives. Preserve the existing GPUI Fast engine, AccessKit fix, and hot-reload setup. Do not undertake another dependency migration.
A source audit and an independent component-research pass found:
1. Changes already uses GPUI Kit’s Diff component and retained DiffState per file. Do not replace it with a homemade diff renderer.
2. The app enables gpui-fast and inspector, but no syntax-highlighting provider. Diff’s syntax_highlight setting alone is insufficient.
3. Changes forces soft wrapping and uses a horizontally scrolling strip of full-path buttons for file navigation.
4. Output flattens structured ACP updates into a single String before rendering plain text. Message boundaries, tool identity, and richer content are lost.
5. Assignment uses a retained TextareaState and Textarea, with no Markdown reading/preview surface.
6. History uses simple text/button rows, hides useful metadata, and requests only the first 30 native revisions without exposing pagination.
7. Current review is not invalidated when a run finishes and records a new revision.
8. Project-row right-click shading may come from generic button active-state handling. Persistent dropdown-trigger shading while a menu is open is intentional; do not remove all pressed/open styling.
9. Custom graph geometry uses fixed pixels while typography scales. Verify zoom behavior and correct demonstrated clipping/alignment issues without expanding this into a graph rewrite.
Reconfirm these against the current checkout before changing them.
Relevant starting points:
- crates/waypoints-app/Cargo.toml
- crates/waypoints-app/src/node_workspace.rs
- crates/waypoints-app/src/node_runs.rs
- crates/waypoints-app/src/workspace.rs
- crates/waypoints-app/src/workflow_graph.rs
- crates/waypoints-app/src/workspace_tests.rs
- docs/20-core-backend-status.md
- docs/05-ui-and-diffs.md
Keep the existing Kit Diff component and its native selection/copy/scroll behavior.
Improve:
- Enable a supported syntax-highlighting provider. Evaluate syntect first for broad readonly display; use narrowly selected Tree-sitter features if there is a concrete reason. Verify actual language coverage and cost rather than assuming.
- Replace the horizontal file-button strip with compact, scannable changed-file navigation suitable for many files and long paths.
- Show useful change status/counts where the model supplies them.
- Provide wrap and unified/split display choices using supported component APIs.
- Choose a readable default; do not force all code to wrap.
- Make the exact comparison target obvious: current recorded revision, saved run snapshot, or native revision.
- Treat commit identity as secondary metadata, with a way to access/copy the full value.
- Preserve selected file and per-file scroll where the target remains the same.
- Give the diff viewport ownership of its own scrolling. Avoid nested scrolling around it.
- Handle empty patches, errors, renames, binary files, conflicts, and unknown languages honestly.
Correct freshness:
- When a run records a new revision, mark or refresh the Current target appropriately.
- Do not silently replace a deliberately pinned run snapshot or historical revision.
- Do not refresh repeatedly during live output or reset the user’s position without cause.
- Preserve existing stale-response generation guards.
Verify with Rust, JavaScript/TypeScript, JSON, unknown extensions, long lines, long paths, many files, and a large patch. Confirm syntax highlighting actually appears in both themes.
Do not merely wrap the existing flattened output String in a Markdown renderer.
Retain a small structured presentation model:
- Keep assistant prose, thought/reasoning events when supplied, tool activity, and system/status notices semantically distinct.
- Assemble streaming chunks into the appropriate block.
- Update an existing tool item when its status/content changes instead of creating disconnected duplicate lines.
- Preserve supported content and retain an inspectable fallback for unknown/non-text events.
- Keep replay labels and output-loss notices outside Markdown content.
- Preserve bounded memory and explicit gap/truncation notices. Trim at coherent block boundaries where practical.
- Do not invent a permanent transcript archive or imply that expired backend events are recoverable.
Presentation:
- Use selectable Markdown for assistant prose, with readable code blocks and copy behavior.
- Use compact tool rows with a clear name, status, and summary; disclose detailed input/output on demand.
- Distinguish thought content from final answers without making the whole view noisy.
- Use subdued structure, not nested cards and multicolored badges everywhere.
- Keep tool details and code blocks usable with long content.
- Maintain a clear current run state, cancellation path, and separate snapshot outcome.
Evaluate Kit TextView and MessageScroller against the locked APIs. Retained TextView streaming APIs exist in 0.9.0, but do not assume live-doc shorthand methods exist unchanged.
Scrolling:
- Follow new output when the user is at the tail.
- Do not pull the user away when they scroll upward to read.
- Offer a clear way to return to latest output.
- Preserve reading position when a tool row updates or expands.
- Keep node switching and background updates isolated.
Use Zed/Delta as reading-quality references, not as justification for importing their architecture. Public Zed references can inform patterns; do not claim knowledge of Delta internals.
Keep the existing textarea, draft retention, dirty detection, save guards, and inline errors.
Add an explicit Edit / Preview affordance:
- Edit shows Markdown source.
- Preview renders the current draft as selectable Markdown.
- Unsaved status remains clear in both modes.
- Switching modes does not discard edits or unnecessarily reset editing position.
- Saving does not change the instructions of an already-active run.
- Saved assignment and follow-up messages remain distinct concepts.
Verify headings, lists, links, inline code, fenced code, long content, empty assignments, save errors, edits made while a save is pending, and switching between nodes.
Do not attempt a complex WYSIWYG editor.
History needs structured information hierarchy, not Markdown decoration.
Use available model data to show:
- Meaningful native operation descriptions.
- Timestamps in a readable, consistent form.
- Run state and snapshot state as distinct facts.
- Current-head and conflict indicators where applicable.
- Associated run references.
- Short revision IDs as secondary monospace metadata, with full identity accessible.
Keep runs and native revisions conceptually distinct. Do not invent commit titles, summaries, or relationships absent from the data.
Fix native revision pagination:
- Expose Load older revisions using the backend’s opaque continuation/pinned-operation contract.
- Distinguish partial history from exhausted history.
- Preserve already-loaded entries if loading another page fails.
- Handle retry, refresh, node switching, and late responses safely.
- Use virtualization where the accumulated collection warrants it.
Keep exact snapshot/revision review readonly. Restore, rebase, deletion, and broader history mutation are outside this task.
Reproduce the project-button right-click issue before changing it.
Test the project disclosure button and ellipsis menu separately:
- Right-button down and release.
- Left-button activation.
- Whether expansion changes.
- Whether a menu appears.
- Whether shading clears after release.
- Menu Escape and outside dismissal.
- Focus restoration.
Distinguish transient pressed feedback, intentional menu-open feedback, and a genuinely stuck state. Fix the demonstrated problem at the narrowest correct layer. Do not globally remove active styling or patch the engine merely to hide a local symptom.
If adding a project context menu, reuse the same commands, enabled states, and owner methods as the visible ellipsis menu. Keep the visible command path.
Also review:
- Keyboard reachability and visible focus.
- Selected versus hovered versus focused states.
- Overlay dismissal and return focus.
- Disabled controls and explanatory errors.
- Shared alignment, insets, density, and scrollbar ownership.
- Light/dark contrast.
- Graph scaling at larger UI font sizes.
Address concrete adjacent defects, but do not turn this into an unrelated application rewrite.
Use an isolated disposable project/repository and isolated backend data/socket/workspaces. Do not run agents or mutate history in the developer’s real project just to demonstrate the UI.
Exercise:
1. Read a substantial assignment in Preview.
2. Edit it, switch nodes and return, then save without losing the draft.
3. Start a real configured agent run.
4. Watch formatted assistant output and tool updates.
5. Scroll upward while output continues; verify no forced jump.
6. Open another node’s saved snapshot, choose a file, and scroll the diff.
7. Receive a permission request without losing navigation.
8. Open and answer the actual request.
9. Return to the same review target/file/position.
10. Finish a run, then verify Current review freshness while historical targets remain pinned.
11. Browse and load older history beyond 30 revisions.
12. Resize/collapse the graph and narrow the window.
13. Complete essential navigation and overlay interactions by keyboard.
If a real harness, trusted executable, credentials, or backend capability is missing, report the exact blocker. Fixture-driven rendering is useful, but do not label it real end-to-end agent validation. Never grant broad permissions or execute destructive commands solely for a demo.
Add focused tests for:
- Structured event assembly and tool updates.
- Replay/live separation, unknown content, partial code fences, Unicode, and bounded trimming.
- Current-review invalidation versus pinned historical targets.
- Pagination, retry, refresh, and stale responses.
- Assignment mode/draft/save behavior.
- Project right-click and menu dismissal where reproducible.
- Meaningful component interactions, not only direct calls to internal methods.
Run:
- cargo fmt --all -- --check
- cargo build -p waypoints-app --locked
- cargo test --workspace --locked
- cargo clippy -p waypoints-app --all-targets --all-features --locked -- -D warnings
If dependency features require a lockfile update, perform that deliberately, review the diff, then run the locked checks.
Never run:
cargo check -p waypoints-app --features hot-reload
Use ./dev for the native development workflow. Restart after dependency, retained-state structure, or startup changes; hot patching alone is not sufficient for those changes.
Start with a brief milestone plan, then implement one surface at a time. Inspect the running native application throughout, not only at the end.
Preserve the good existing behavior:
- Draft and per-node state retention.
- Retained per-file DiffState.
- Async generation guards.
- Honest output-loss notices and bounded memory.
- Semantic components, theme tokens, resizable regions, and permission isolation.
At completion, provide:
- A concise summary of changes.
- Representative actual screenshots of Changes, Output, Assignment preview, and History.
- Which acceptance steps were completed, with any blockers.
- Exact automated checks and their results.
- Remaining limitations or deferred issues.
Do not commit, push, or merge unless explicitly requested.
The goal is a quiet, readable, trustworthy native workspace—not more decoration, more controls, or pixel-perfect imitation of another app.
