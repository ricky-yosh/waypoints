# Waypoints
Chaining prompts by hand means copy-pasting outputs, losing track of context, and not knowing which results still hold after you change something upstream. [Name] makes the plan the main thing you work with.

Plans as blocks: phases made up of points, written as a document you can edit.
Agents you already use: each phase runs in an ACP-compatible agent, with resumed or forked sessions where the agent supports them.
A snapshot per phase: every phase is a jj change, so you can see what it introduced, run alternatives in parallel, and merge them.
Stale and conflict tracking: edit an early phase and everything downstream is marked, with jj conflicts shown instead of hidden.
Review without rebuilding an editor: inline summaries and Markdown in the app, with a one-click hand-off to Zed for deep diff review.

Local-first, written in Rust, with GPUI for the interface and an embedded database for state.
