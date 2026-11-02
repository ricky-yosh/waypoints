# Stack and storage

## Rust core

Workflow logic, storage, and jj integration live in a core library with no UI dependency. A local backend hosts that library and exposes the JSON-RPC API used by the CLI and intended for the GUI. The GUI must use the backend client, not shell out to CLI workflow commands or open the store directly. The backend owns database access and coordinates Waypoints-initiated operations; GUI integration is not implemented.

The CLI is the interface for human scripting and agents with terminal access; no MCP adapter is planned. Keep workflow metadata durable, but leave conversations and transcripts in existing harnesses. Current clients poll bounded run events and refresh saved state after reconnect; subscriptions and durable event replay are not implemented or required. See the [architecture decision](02-architecture.md#architecture-decision-local-backend-gui-and-cli) for ownership and the [current status map](20-core-backend-status.md) for implemented boundaries.

## GPUI

The app manifest selects `gpui-kit`, pinned to 0.8.1 with inspector support.
It reexports GPUI and provides the component layer; do not add separate GPUI or
`gpui-component` dependencies by copying older research examples. The current
app executable is still a placeholder, not a backend-connected desktop client.
Use the installed GPUI Kit coding/design guides and verify APIs against the pin.

## Storage: Turso

- The core uses the Rust `turso` crate (manifest requirement 0.8.3) for a local file-backed database. See `Cargo.lock` for the resolved dependency version.
- One backend owns the configured database/socket pair and holds lifetime ownership locks. Do not open the live database through another client or an external SQLite CLI; use the supported API.
- Keep SQL plain where practical. SQLite-compatible storage does not establish a tested drop-in driver migration, live-file-copy backup, or downgrade path.
- Backup/restore procedures, general migration policy, and relocation remain discussion items. The targeted restore-journal upgrade is documented in [restoration](21-saved-result-restoration.md#api-and-persistence); do not downgrade against newer records.

## Prototype language

- React + TypeScript + Tailwind with React Flow was considered for an optional UI prototype. It is not an adopted runtime dependency or a prerequisite for the native GUI.
- Do not translate code. Translate decisions: the data model, the state machine, layouts and interactions you like, and the shape of agent events. Have the agent write these down as a short spec for the GPUI build.
- Make the prototype honest: mock data, a fake agent that emits events shaped like ACP `session/update`, and only interactions feasible in GPUI.

## Backend: prototype or just build?

Build your own code for real (plan model, state machine, SQL). Spike only the parts that depend on programs you do not control. See `07-spikes-and-build-order.md`.
