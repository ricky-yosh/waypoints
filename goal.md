# Goal: Iteratively audit and polish the Waypoints native experience

Improve Waypoints through repeated, evidence-based UI/UX loops. Each loop should select a small set of related user-facing problems, reproduce them, implement the narrowest appropriate fixes, and verify the result in the running native application. Do not attempt to inventory every possible improvement or redesign the product in one pass.

This supersedes the earlier one-pass node-workspace polish brief. Capabilities described in that brief and the current docs are baseline behavior to preserve and re-evaluate, not a list to implement again.

The goal is a quiet, readable, trustworthy native workspace. A plausible screenshot, a passing build, or a completed checklist alone is not evidence of a good workflow.

## Preserve the product and architecture

Start with the current source, tests, and desktop documentation. Older placeholder and readonly-browser descriptions are historical where they disagree with the current app.

Preserve the existing structure unless a concrete defect requires a narrowly scoped change:

- Project/workflow navigation in the sidebar.
- Compact horizontal dependency graph above the node workspace.
- Resizable and collapsible graph and sidebar regions.
- Full-width node workspace with Assignment / Output / Changes / History.
- Per-node draft, tab, selected review target/file, and scroll retention.
- Permission requests that never steal navigation.
- Structured, bounded output with honest replay, gap, and truncation notices.
- Kit's Diff component and retained per-file DiffState.
- Current versus pinned historical review behavior and async generation guards.
- Existing backend ownership, GPUI Fast engine, AccessKit fix, and hot-reload setup.

Keep run state, snapshot outcome, native revision, workflow metadata history, agent session, and physical checkout state conceptually distinct. Tool permission is not code-review approval. Do not add inferred agent-context-staleness states, automatic reruns, a transcript archive, or silent recovery behavior as UI polish.

Use GPUI Kit components before inventing primitives. Read the GPUI Kit Design Guides and Coding Guides before UI or architecture decisions, and verify APIs against the locked component source. Do not undertake a dependency migration.

## Use a goal loop, not a one-time exhaustive audit

Repeat the following loop with one coherent task or surface at a time:

1. **Choose a task.** Select a real user task or a small related surface, based on observed friction or risk. Examples include opening and navigating a workflow, editing an assignment, following a run, reviewing a patch, browsing history, answering a permission request, or resizing the workspace.
2. **Establish the baseline.** Name the primary task, object, expected next action, relevant states, and keyboard path. Read the nearest implementation and tests. Distinguish current behavior from assumptions in older docs.
3. **Audit with evidence.** Inspect the current native app and source using the checklists below. Reproduce suspected problems where practical. Record what happened, the expected behavior, and why it matters. Do not manufacture a finding for every heuristic.
4. **Prioritize a small batch.** Group one to a few findings with a shared user problem or root cause. Prefer task blockage, misleading state, data-loss risk, inaccessible interaction, and lost context over cosmetic preferences.
5. **Implement narrowly.** Fix the structural owner rather than adding local patches around symptoms. Preserve working behavior and avoid unrelated refactors, new controls, or new decoration.
6. **Verify the workflow.** Exercise the real interaction with pointer and keyboard where applicable. Test representative dynamic content and failure states, not only an ideal fixture. Add focused regression coverage at the lowest layer that proves the behavior.
7. **Review and choose the next loop.** Mark findings verified, unresolved, rejected, or deferred with a reason. Record newly discovered issues without expanding the current batch. Stop the loop when its selected task is demonstrably improved; do not keep polishing merely because more checklist rows exist.

A loop may be audit-only when implementation is not yet justified. A loop may also reject a suspected issue after reproduction shows that the behavior is correct.

## Maintain lightweight audit evidence

Keep a living audit record when useful, without turning it into a comprehensive product wishlist. Each actionable finding should capture:

- Task and surface.
- Observed behavior and reproduction conditions.
- Expected behavior and user impact.
- Relevant heuristic or product contract.
- Severity and confidence.
- Evidence: native interaction, screenshot, accessibility observation, source reference, or test.
- Proposed direction, without prematurely prescribing a redesign.
- Status and verification evidence after a fix.

Use these severity meanings:

- **High:** blocks or misleads a core task; risks work; conceals freshness or scope; prevents keyboard access; causes destructive or irreversible surprise.
- **Medium:** repeated friction, lost context, weak recovery, unclear hierarchy, inconsistent interaction, or significant accessibility/theme/resize problems.
- **Low:** localized consistency, alignment, density, copy, or refinement issues with a clear user benefit.

Separate confirmed defects, hypotheses still needing reproduction, intentional constraints, and optional enhancements. Do not present a backend limitation or safety refusal as a defect merely because it adds friction.

## Reusable heuristic checklist

Use these prompts as lenses during each loop, not as a requirement to answer every question every time.

### Visibility of system status

- Can the user tell what is loaded, selected, running, stale, saved, unavailable, or waiting?
- Are progress and completion attached to the correct object and operation?
- Does feedback appear promptly without pretending that acknowledgement is completion?
- Are run outcome, snapshot outcome, and review freshness independently understandable?

### Match with the user's mental model

- Does the UI emphasize projects, workflows, assignments, output, code, and history rather than backend mechanics?
- Are the exact object, comparison target, scope, and consequence visible before an action?
- Are native jj terms used only where they add precision, with honest distinctions between change, revision, run, and session?

### User control and freedom

- Can users navigate away, dismiss overlays, cancel appropriate work, and return without losing drafts or position?
- Do streaming output and background updates avoid pulling the user away from their current reading or review?
- Are reversible actions easy to reverse, and consequential actions explicit about their scope and undo boundary?
- Does the app avoid blind retry, duplicate submission, automatic repair, and unexpected focus theft?

### Consistency and standards

- Do equivalent commands share labels, icons, enabled states, shortcuts, and results across buttons and menus?
- Are rest, hover, pressed, open, focused, selected, disabled, loading, readonly, validation, and destructive states distinct and consistent?
- Do desktop conventions, platform shortcuts, window chrome, and menu behavior remain intact?
- Are standard GPUI Kit components used for their semantic behavior rather than approximated with clickable containers?

### Error prevention and recovery

- Are unavailable actions hidden or disabled with a useful explanation where needed?
- Do errors say what happened, preserve user input and existing data, and offer a realistic next action?
- Are empty, loading, partial, offline, permission-denied, failed, expired, and interrupted states handled honestly?
- Are stale responses, late async completions, refresh, retry, and node switching prevented from corrupting current state?

### Recognition rather than recall

- Are current selection, primary action, next step, and recovery path visible without memorizing prior screens?
- Do truncated paths, identifiers, and labels have a way to reveal or copy the full value?
- Do icon-only controls have accessible names and tooltips when their meaning or scope is unclear?

### Flexibility and efficiency

- Are frequent actions visible, secondary actions appropriately grouped, and important commands keyboard reachable?
- Are user-controlled layout, per-node context, and reading positions retained where they help the task?
- Are dense collections scannable and progressively disclosed without hiding essential actions only on hover?

### Minimalist and precise presentation

- Does content receive more emphasis than chrome, decoration, badges, alerts, and containers?
- Is there one clear reading order and decision path, with emphasis reserved for information that changes action?
- Are alignment spines, spacing relationships, control sizes, separators, and scrollbar placement consistent?
- Are theme tokens and rem-based sizing used instead of isolated colors or geometry, except for documented physical boundaries?

### Accessibility and resilience

- Is every essential action operable by keyboard, with logical focus order, visible focus, and focus restoration after overlays?
- Is status communicated by more than color alone, with adequate contrast in both themes?
- Does the interface remain usable at minimum and narrow window sizes, larger UI font sizes, and with long labels, paths, output, and history?
- Are scrolling, text selection, copy, virtualization, reduced motion, and pointer targets appropriate to the task?

### Waypoints-specific trust

- Is the exact Current, saved-run, or native-revision review target obvious, and are pinned targets kept pinned?
- Are replay, expired events, bounded output, and unavailable native objects described without implying a complete permanent transcript?
- Do permission requests remain inspectable and answerable without losing the underlying workspace context?
- Are readonly inspection, restoration, reparenting, deletion, recovery, and checkout updates represented as distinct operations?
- Does the UI avoid claiming that saved IDs pin native objects forever or that a clean rebase proves behavioral correctness?

## Validation expectations for each implementation loop

Use an isolated disposable project/repository and isolated backend data, socket, preferences, and workspaces. Do not mutate the developer's real project or history for a UI demonstration. Use fixture-driven rendering for layout and state coverage, but do not label it real end-to-end agent validation.

Exercise the states and constraints relevant to the selected findings:

- Pointer and keyboard interaction, including focus and overlay dismissal.
- Existing content during refresh, slow operations, failure, cancellation, and retry where applicable.
- Light and dark themes for visual changes.
- Minimum/narrow and comfortable window sizes, graph/sidebar resizing, and UI zoom where layout is involved.
- Long, empty, numerous, unknown, and partial content appropriate to the surface.
- Before/after native captures when they clarify a visual fix; never screenshots alone as acceptance.

Run focused tests during the loop. Before declaring the overall goal complete, run:

- `cargo fmt --all -- --check`
- `cargo build -p waypoints-app --locked`
- `cargo test --workspace --locked`
- `cargo clippy -p waypoints-app --all-targets --all-features --locked -- -D warnings`

Never run `cargo check -p waypoints-app --features hot-reload`. Use `./dev` for native hot reload, restarting after dependency, retained-state structure, or startup changes.

If a real harness, trusted executable, credentials, or backend capability is required but unavailable, report the exact blocker. Never grant broad permissions or execute destructive commands solely for a demo.

## Loop and overall completion

After each loop, report:

- The task and findings selected.
- Evidence that the problems were real.
- Changes made and anything deliberately left unchanged.
- Automated and native interaction checks completed, with blockers.
- Findings verified, deferred, rejected, or discovered for a later loop.

The overall goal is complete when representative core workflows meet this quality bar, no known high-severity findings remain unaddressed, and remaining lower-severity opportunities are explicitly deferred. It is not necessary to exhaust every heuristic or eliminate every possible refinement.

Do not commit, push, or merge unless explicitly requested.
