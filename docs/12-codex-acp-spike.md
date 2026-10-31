# Codex ACP spike: observed results

## Outcome and scope

The local spike passed with `@agentclientprotocol/codex-acp` **2.5.0**, negotiating
ACP protocol version **1**. It reused the existing ChatGPT login without new
credentials.

This was a temporary Python ACP client, not production agent-execution support
in Waypoints. The disposable workflow, node, change, and dedicated jj workspace
were created through the existing Waypoints CLI. All agent edits were confined
to that test workspace; the original workspace files stayed unchanged.

No production source changes, dependency installation, commits, or pushes were
part of the spike. The test adapter and backend processes were stopped afterward.
The agent may retain the test conversations in its normal local session store.
The temporary client and raw events were scratch artifacts, not a checked-in
test harness; this document preserves the observations.

## Connection and installation

Waypoints plays the same ACP **client** role as Zed:

```text
Waypoints ↔ ACP adapter ↔ Codex App Server ↔ model service
```

Zed does not need to be installed or running. ACP is the agent communication
protocol, not a model API or authentication system. The agent handles the model
calls and coding-agent loop; Waypoints sends prompts, handles callbacks, and
tracks the resulting native jj changes.

The current adapter package is `@agentclientprotocol/codex-acp`; the older
`@zed-industries/codex-acp` repository directs new installs there. The npm package
includes a compatible Codex dependency, so the existing Codex installation did
not need modification.

With Node.js and npm available, install from any directory:

```sh
npm install -g @agentclientprotocol/codex-acp
codex-acp --version
command -v codex-acp
```

These commands install the current published release, not necessarily the tested
version. For reproduction, pin `@agentclientprotocol/codex-acp@2.5.0`.
Installation and authentication are separate. The observed saved ChatGPT login
worked with this adapter, but that is not a promise about every installation,
provider, or environment. Do not put credentials in project files.

Running bare `codex-acp` waits for an ACP client, rather than opening an
interactive chat. In this installation, `codex-acp --help` also waited and was
stopped by a timeout; `--version` worked.

For the first production integration, use a user-installed executable. Managed
downloads can be a later feature; they are not necessary to prove the connection.

## Checks performed

| Check | Observed result |
|---|---|
| Initialize | Negotiated protocol version 1 and received capabilities, authentication methods, modes, and model configuration |
| Authentication | `session/new` worked with the existing ChatGPT login, without another login flow |
| Streaming | Received incremental assistant text, tool calls/updates, thought chunks, and usage events |
| File edit | Codex changed `greeting.txt` from `Hello from the baseline.` to `Hello from Codex over ACP.`; contents were verified on disk |
| Structured diffs | Received `diff` content with paths, `oldText`, and `newText`; the replacement arrived as delete/add events |
| Same-session follow-up | A second prompt recalled a token supplied only in conversation, without reading files |
| Restart and load | Restarted the adapter and called `session/load` with the same session ID and working directory; history replayed, and a new prompt recalled the token and edited filename |
| Cancellation | Sent `session/cancel` after tool activity for a requested `sleep 30`; the prompt returned `stopReason: cancelled` |
| Usability after cancellation | A subsequent prompt completed normally in the same session |
| Denied permission | In read-only mode, a file edit triggered `session/request_permission`; selecting `reject_once` returned a cancelled turn and left the proposed file absent |
| Approve once | Retrying that single-file edit triggered another permission request; selecting `allow_once` created `permission-test.txt` with the expected contents |
| Native jj integration | Explicit `jj status` snapshotted only the disposable node workspace; `jj diff --git` matched the two file changes |
| Waypoints refresh | `node refresh` kept the node's associated change ID and observed a different commit ID after that snapshot |
| Workspace isolation | Original workspace file contents were unchanged and the approval-test file was absent there |

The adapter advertised `loadSession` and additional session capabilities. Only
`session/load` was exercised for restart continuation; advertisement alone is not
evidence that resume, fork, or other operations work.

## Integration rules learned

### Proposed diffs are not completed edits

A diff was emitted for the read-only edit **before approval**. Denying the
request left the file absent. Do not present every diff event as a successful
filesystem change. Keep proposed content separate from tool completion and
native repository state.

Also, do not assume one edit produces one modify event: the greeting replacement
arrived as a deletion and an addition for the same path.

### Session creation is not proof of durable continuation

A session created and then disconnected before any prompt could not be loaded
after restarting the adapter: `no rollout found for thread id`.

Loading worked after actual conversation turns. Do not promise durable
continuation merely because `session/new` returned an ID. Handle load failure
explicitly rather than silently presenting a fresh conversation as a resumed one.

History replay from `session/load` must not be treated as new live activity or
new edits.

### Select permission mode explicitly

The existing user configuration initially selected Auto review (`agent`) mode.
The spike explicitly selected `workspace-write` for the ordinary edit and
`read-only` for approval tests through `session/set_mode`.

Production should select and surface the intended mode, not inherit an unknown
default or silently request full access. Approve-once and denial were tested;
blanket session approval was not.

### Cancellation has a bounded claim

The protocol returned `cancelled` and accepted a subsequent prompt. The spike
did not independently verify that every descendant process had terminated.
Process-lifecycle cleanup needs its own validation.

### Agent edits and native snapshots are separate

The spike explicitly invoked native `jj status` before refreshing the node.
This is not evidence that read-only Waypoints refresh snapshots agent edits.
Production must make the snapshot boundary deliberate while preserving the
existing read-only refresh contract.

## Not tested

- Loading the same session from a different working directory.
- `session/resume` without replay, fork, list, close, or deletion semantics.
- Network approvals, client filesystem/terminal callbacks, or MCP integration.
- Large or binary diffs, moves, conflicting edits, or long-running workloads.
- Crash recovery or independent descendant-process termination.
- Other agent adapters, model providers, or fresh authentication flows.
- Production Rust ACP integration or GUI integration in this temporary-client spike.
  A later manual test validated the Rust fresh-run path; see the lifecycle document.

## Next production slice

The subsequent [backend-owned lifecycle checkpoint](13-agent-run-lifecycle.md)
implements fresh-session starts, bounded polling, approvals, and cancellation,
plus explicit session loading/continuation with replay markers, validated with a
fake agent. Manual Codex tests validated the Rust fresh-run/file-edit path and
continuation. Automatic snapshot finalization also passed the manual Codex check.
Explicit reservation recovery is subsequently fake-agent/native-jj tested; it
does not reconstruct interrupted run revisions or automatically replay work.
The list below records the
full target derived from this spike, not a claim that every item is implemented.

Support one installed Codex adapter and one node workspace:

1. Launch the configured executable and negotiate capabilities.
2. Create a session for the node workspace and send the saved prompt.
3. Stream text and tool events, distinguishing proposed diffs from completed work.
4. Surface permission requests and cancellation with explicit permission modes.
5. Retain the harness-owned session reference and explicitly load it for
   continuation, handling replay and load failures.
6. Define the native snapshot boundary and refresh the node's jj association.

Do not add a custom agent loop, one-shot `codex exec` fallback, transcript archive,
managed installer, or multi-agent framework to this slice.

## References

- [Current Codex ACP adapter and setup](https://github.com/agentclientprotocol/codex-acp)
- [Archived adapter migration notice](https://github.com/zed-industries/codex-acp)
- [ACP initialization](https://agentclientprotocol.com/protocol/initialization)
- [ACP session setup and loading](https://agentclientprotocol.com/protocol/session-setup)
- [Waypoints ACP design](03-acp-and-agents.md)
