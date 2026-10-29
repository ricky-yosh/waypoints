# Stack and storage

## Rust core

Plan model, state machine, jj and ACP plumbing live in a core crate with no UI dependency. Build it as a library with a thin CLI first (create a plan, run a phase, see state, print diff). The CLI becomes the test harness and what the UI calls later.

## GPUI

GPUI (from the Zed team) is a hybrid immediate and retained mode, GPU-accelerated UI framework for Rust. It is pre-1.0 with frequent breaking changes and needs recent stable Rust. Use `gpui-component` for ready-made widgets.

## Storage: Turso

- Turso database is a from-scratch Rust rewrite of SQLite; Rust crate `turso` (`cargo add turso tokio`).
- It is in beta. Caveats noted: only one process can open the file at a time (so no sqlite3 CLI poking while the app runs), no VACUUM, back up the file occasionally.
- It uses a SQLite-compatible file format. Keep SQL plain so switching to SQLite is a small change.

## Prototype language

- Use React + TypeScript + Tailwind, with React Flow for the node canvas. AI agents write React fluently, React Flow gives pan/zoom/drag/edges immediately, and Tailwind-style layouts map loosely to GPUI's CSS-like flexbox styling.
- Do not translate code. Translate decisions: the data model, the state machine, layouts and interactions you like, and the shape of agent events. Have the agent write these down as a short spec for the GPUI build.
- Make the prototype honest: mock data, a fake agent that emits events shaped like ACP `session/update`, and only interactions feasible in GPUI.

## Backend: prototype or just build?

Build your own code for real (plan model, state machine, SQL). Spike only the parts that depend on programs you do not control. See `07-spikes-and-build-order.md`.
