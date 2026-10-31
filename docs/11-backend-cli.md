# Local backend and CLI

## Quick start

No database server or manual backend startup is required:

```sh
cargo run -p waypoints-cli -- workflow create --project . --name "First workflow"
cargo run -p waypoints-cli -- workflow list --project . --json
cargo run -p waypoints-cli -- backend status --json
cargo run -p waypoints-cli -- backend stop
```

`--json` prints a single JSON value on stdout. Errors go to stderr and exit
nonzero; command syntax errors use Clap's exit status 2. A workflow name may be
duplicated, but it must not be blank. Projects must be existing directories; both
clients and storage resolve their paths so relative paths don't depend on the
background server's working directory.

## Data and socket paths

Defaults:

- If set, `$XDG_STATE_HOME/waypoints/workflows.db`.
- Otherwise on macOS, `$HOME/Library/Application Support/waypoints/workflows.db`.
- Otherwise on Linux, `$HOME/.local/state/waypoints/workflows.db`.

The default socket is alongside the database, named `workflows.sock`. Newly
created parent directories have mode 0700; the socket and newly created lock/log
files have mode 0600. Existing directories are not chmodded.

For isolated development data, consistently pass the same overrides to every
command:

```sh
cargo run -p waypoints-cli -- --database ./dev-data/workflows.db --socket ./dev-data/backend.sock workflow create --project . --name "Test"
cargo run -p waypoints-cli -- --database ./dev-data/workflows.db --socket ./dev-data/backend.sock workflow list --project . --json
cargo run -p waypoints-cli -- --database ./dev-data/workflows.db --socket ./dev-data/backend.sock backend stop
```

Unix socket paths have a short OS-dependent length limit (about 100 bytes on
macOS). If necessary, choose a shorter socket path in a directory you own.
Local development data is not source code: do not commit database, socket, lock,
or log files.

## Lifecycle

Workflow commands first connect and verify the database path and protocol version.
Only a missing or refused socket triggers a background `waypoints … serve`
process. A live but unresponsive or mismatched backend produces an error instead
of being replaced. Readiness waits are bounded; startup errors are written to
`<socket>.log`. The server runs independently of the CLI's terminal process group
and remains available until stopped or interrupted.

The server holds advisory locks at `<database>.lock` and `<socket>.lock` before
opening storage or touching an existing socket. Concurrent starters may briefly
launch competing processes, but only one opens the database; the others exit and
their clients connect to the winner. Lock files are deliberately never unlinked.
Locks release when the process exits, including after a crash.

A stale socket is removed only after acquiring both ownership locks and checking
that it refuses connections. A regular file, symlink, or live listener at that path
is not removed. `backend status` and `backend stop` do not start a backend.
Shutdown normally removes the socket; the database and lock/log files remain.

For foreground debugging only:

```sh
cargo run -p waypoints-cli -- serve
```

Ctrl+C stops a foreground server. This isn't a system service, and there is no
automatic idle shutdown yet. Stop/restart a background backend after rebuilding
its executable to ensure you're running the new code.

## API

The `waypoints-backend` library provides the shared configuration, client,
connect-or-start logic, and server. A future GUI can use the same client while
launching the companion `waypoints` executable; the GUI does not call CLI workflow
commands or access the database.

Transport: a local Unix socket, one newline-delimited JSON-RPC 2.0 request per
connection. Clients reconnect for each request. The backend serializes requests.
Frames are limited to 1 MiB, and socket reads/writes have five-second timeouts.
Storage mutations are not cancelled by a server-side transport timeout. Requests
are not automatically retried: a lost response may follow a successful mutation.

Methods:

| Method | Parameters | Result |
|---|---|---|
| `backend.info` | `{}` | `{pid, database, protocol_version}` |
| `backend.shutdown` | `{}` | `null` |
| `workflow.create` | `{project_path, name}` | `{id, project_path, name}` |
| `workflow.list` | `{project_path}` | Array of workflows ordered by ID |

Example:

```json
{"jsonrpc":"2.0","id":1,"method":"workflow.list","params":{"project_path":"/absolute/project/path"}}
```

Syntax errors, invalid requests, unknown methods, and invalid parameter shapes use
JSON-RPC codes -32700, -32600, -32601, and -32602. Storage/domain failures use
-32000 with a descriptive message. Notifications receive no response. Batch
requests and subscriptions are not supported. Oversized or incomplete frames
close the connection.

## Checks

```sh
cargo test -p waypoints-core -p waypoints-backend -p waypoints-cli
cargo clippy -p waypoints-core -p waypoints-backend -p waypoints-cli --all-targets --all-features -- -D warnings
cargo doc -p waypoints-core -p waypoints-backend -p waypoints-cli --no-deps
cargo fmt --check
```

End-to-end tests launch the real executable and background processes with isolated
temporary databases, then stop the backend during cleanup.

## Deliberately not included

- Windows named pipes (the backend currently requires macOS/Linux).
- GUI integration, notifications, or durable event replay.
- Project registration, workflow nodes/links, undo history, jj mutations, or ACP.
- Database migrations beyond the initial create-if-missing schema.
