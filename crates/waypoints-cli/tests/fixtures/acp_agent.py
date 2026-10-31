#!/usr/bin/env python3
"""Deterministic ACP peer for lifecycle integration tests; never calls a model."""
import json
import os
from pathlib import Path
import subprocess
import sys


def send(message):
    print(json.dumps({"jsonrpc": "2.0", **message}), flush=True)


def update(value):
    send({"method": "session/update", "params": {"sessionId": "test-session", "update": value}})


def done(reason):
    send({"id": 4, "result": {"stopReason": reason}})


scenario = ""
loading = False
mode_selected = False
memory = None
# Fake harness-owned history and protocol trace, outside the node workspace.
history = Path(__file__).with_suffix(".session.json")
trace = Path(__file__).with_suffix(".requests.jsonl")
for line in sys.stdin:
    message = json.loads(line)
    method = message.get("method")
    with trace.open("a") as log:
        log.write(json.dumps(message) + "\n")
    if method == "initialize":
        Path("adapter.pid").write_text(str(os.getpid()))
        if Path("stall-initialize").exists():
            continue
        capabilities = {"loadSession": True}
        if Path("load-capabilities.json").exists():
            capabilities = json.loads(Path("load-capabilities.json").read_text())
        send({"id": 1, "result": {"protocolVersion": 1, "agentCapabilities": capabilities}})
    elif method == "session/new":
        assert message["params"]["cwd"] == str(Path.cwd())
        history.write_text(json.dumps({"memory": None}))
        send({"id": 2, "result": {
            "sessionId": "test-session",
            "modes": {"availableModes": [{"id": "workspace-write"}]},
        }})
    elif method == "session/load":
        loading = True
        assert message["params"]["cwd"] == str(Path.cwd())
        assert message["params"]["sessionId"] == "test-session"
        memory = json.loads(history.read_text())["memory"]
        if Path("load-wrong-update").exists():
            send({"method": "session/update", "params": {"sessionId": "wrong-session", "update": {}}})
            continue
        count = 300 if Path("load-flood").exists() else 1
        for index in range(count):
            update({"sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": memory or f"history-{index}"}})
        update({"sessionUpdate": "tool_call", "toolCallId": "historical-edit",
                "kind": "edit", "title": "Historical edit, not a new request", "status": "completed"})
        if Path("stall-load").exists():
            continue
        loading = False
        if Path("load-error").exists():
            send({"id": 2, "error": {"code": -32000, "message": "no rollout found"}})
        elif Path("load-malformed").exists():
            send({"id": 2, "result": []})
        elif Path("load-wrong-session").exists():
            send({"id": 2, "result": {"sessionId": "wrong-session"}})
        else:
            # ACP allows an empty load result, including no repeated session ID/modes.
            send({"id": 2, "result": {}})
    elif method == "session/set_mode":
        assert message["params"]["modeId"] == "workspace-write"
        if Path("mode-error").exists():
            send({"id": 3, "error": {"code": -32602, "message": "unsupported mode"}})
        else:
            mode_selected = True
            send({"id": 3, "result": {}})
    elif method == "session/prompt":
        assert mode_selected
        scenario = message["params"]["prompt"][0]["text"]
        if scenario.startswith("remember "):
            memory = scenario.removeprefix("remember ")
            history.write_text(json.dumps({"memory": memory}))
            Path("hello.txt").write_text("Hello from the first turn.\n")
            update({"sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "Remembered; created hello.txt."}})
            done("end_turn")
            continue
        Path("captured-prompt.txt").write_text(scenario)
        if scenario == "large-file":
            Path("large.txt").write_text("x" * (2 * 1024 * 1024))
            done("end_turn")
            continue
        if scenario == "recall and extend":
            assert memory is not None, "must load conversation-only state before prompting"
            update({"sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": f"Recalled: {memory}"}})
            with Path("hello.txt").open("a") as output:
                output.write("Extended in the second turn.\n")
            done("end_turn")
            continue
        if scenario == "malformed":
            print("not JSON", flush=True)
            continue
        if scenario == "exit":
            sys.exit(1)
        if scenario == "oversize":
            update({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "x" * 70000}})
            continue
        if scenario == "hang":
            child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(3600)"])
            Path("descendant.pid").write_text(str(child.pid))
        count = 300 if scenario == "flood" else 1
        for index in range(count):
            update({"sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": f"event-{index}"}})
        if scenario in ("permission", "flood"):
            send({"id": "approval-1", "method": "session/request_permission", "params": {
                "sessionId": "test-session",
                "toolCall": {"toolCallId": "test-edit", "kind": "edit",
                             "title": "Write approved.txt", "status": "pending"},
                "options": [
                    {"optionId": "allow", "kind": "allow_once", "name": "Allow once"},
                    {"optionId": "deny", "kind": "reject_once", "name": "Deny once"},
                    {"optionId": "blanket", "kind": "allow_always", "name": "Allow all"},
                ],
            }})
        elif scenario == "complete":
            done("end_turn")
        # Other scenarios wait for control input instead of finishing.
    elif method == "session/cancel":
        if scenario != "hang" and not loading:
            done("cancelled")
        # "hang" deliberately ignores cancellation to exercise group cleanup.
    elif message.get("id") == "approval-1":
        outcome = message["result"]["outcome"]
        if outcome.get("optionId") == "allow":
            Path("approved.txt").write_text("approved once\n")
            done("end_turn")
        else:
            done("cancelled")
