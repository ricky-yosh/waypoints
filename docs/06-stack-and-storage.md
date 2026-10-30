# Stack and storage

## Rust core

Workflow logic, storage, and jj integration live in a core library with no UI dependency. A local backend hosts that library and exposes a JSON-RPC API shared by the GUI and CLI. The GUI does not shell out to the CLI, and clients do not open independent copies of the store. The backend owns database access and coordinates Waypoints-initiated operations.

The CLI is the interface for human scripting and agents with terminal access; no MCP adapter is planned. Keep workflow metadata durable, but leave conversations and transcripts in existing harnesses. Start with state-change notifications and a full refresh on reconnect rather than event replay. See the [architecture decision](02-architecture.md#architecture-decision-local-backend-gui-and-cli) for the authoritative boundary and local IPC recommendation.

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
