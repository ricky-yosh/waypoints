Goal: Audit and fix UI/UX inconsistencies in the Rune desktop app so
existing workflows feel coherent, readable, and controllable.

Start with the History view: its side-by-side list/details panels need
a draggable divider comparable to the Changes file-list/diff divider.
Inspect and reuse the existing split-pane behavior where appropriate.
Ensure sensible minimum widths and usable behavior in narrow windows.

Then audit the existing app for similar issues:
- Missing or inconsistent resizing and scrolling.
- Clipped text, overflowing controls, and poor narrow-window behavior.
- Selection, keyboard navigation, focus restoration, and Escape behavior.
- Unclear loading, empty, error, disabled, and stale states.
- Inconsistent labels, tooltips, copy feedback, and action placement.
- Lost view context when switching nodes, tabs, or workflows.

Read repository instructions, docs/05-ui-and-diffs.md,
docs/28-agent-led-workflows.md, and the pinned Rune/Rune Kit guidance.
Inspect actual APIs rather than inventing them.

Process:
1. Reproduce the History divider issue and fix it first.
2. Create docs/ui-ux-audit.md with a prioritized findings list.
   Each finding needs a location, reproduction steps, user impact,
   proposed fix, and validation status.
3. Fix high-confidence usability defects in small, coherent batches.
   Reuse established interaction rules rather than patching each example
   differently. Add regression coverage for observable behavior.
4. Render and inspect representative headless scenes in light/dark,
   narrow/normal windows, and larger interface text. Test interactions,
   not screenshots alone. Perform native checks where available.
5. Update the audit after each batch and continue through actionable
   findings. Finish with fixed items, deferred decisions, exact checks,
   and remaining native limitations.

Boundaries:
- Preserve backend contracts, safety checks, and agent-led onboarding.
- Do not add a chat, workflow creation form, or new backend capabilities.
- Do not introduce a new visual language or broadly restructure the app.
- Ask before changes with genuine product tradeoffs; record them and
  continue independent fixes.
- Preserve concurrent edits. Do not commit or push.
- Do not claim native usability was verified from headless tests alone.

Done means the History divider works, actionable high-confidence
findings are fixed and regression-tested, and remaining issues are
explicit—not that the app has been declared universally polished.

