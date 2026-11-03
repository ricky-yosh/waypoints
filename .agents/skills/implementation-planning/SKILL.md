---
name: implementation-planning
description: Turn an ongoing conversation about a software change into a clear, practical implementation plan before coding begins.
metadata:
  short-description: Plan readable, simple implementations
---

# Implementation planning

Use this skill when the user asks to plan implementation after discussing a feature, bug fix, or other software change. Treat the conversation and the current repository as the source of truth.

## Plan briefly

- Account for the user's ADHD: use plain language, short sections, and a few clear steps. Keep the default response concise; include detail only when it changes a decision or prevents a likely problem.
- Summarize the goal, then give a short ordered plan. Split larger work into a few milestones.
- Inspect relevant project guidance and code. Identify useful APIs or patterns to reuse and any code the change makes obsolete. Mention only findings that affect the plan; skip a broad audit.
- Prefer readable names, direct control flow, and the simplest approach that meets requirements. Keep necessary validation and error handling.
- State material risks or unresolved decisions only. Ask only when the answer would change the implementation; otherwise make a clear assumption.
- End with the next step. Planning alone does not authorize implementation.

Planning does not itself authorize implementation. Stop after presenting the plan and let the user direct the next action.
