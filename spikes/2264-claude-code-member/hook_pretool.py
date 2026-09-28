#!/usr/bin/env python3
"""PreToolUse hook: refuse Edit/Write outside the member's board reservations; refuse denylisted Bash.
Reads the hook JSON on stdin; answers with a permissionDecision JSON on stdout. Disposable."""
import json, os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import board

ev = json.load(sys.stdin)
if os.environ.get("SPIKE_HOOK_LOG"):
    with open(os.environ["SPIKE_HOOK_LOG"], "a") as f: f.write(json.dumps(ev) + "\n")
tool, inp = ev.get("tool_name"), ev.get("tool_input") or {}
member = os.environ.get("SPIKE_MEMBER", "W1")

def decide(decision, reason):
    print(json.dumps({"hookSpecificOutput": {"hookEventName": "PreToolUse",
                      "permissionDecision": decision, "permissionDecisionReason": reason}}))
    sys.exit(0)

if tool in ("Edit", "Write", "MultiEdit", "NotebookEdit"):
    path = os.path.normpath(inp.get("file_path") or inp.get("notebook_path") or "")
    reserved = board.reserved_files(member)
    if path in reserved:            # allowlist: only reserved paths pass
        sys.exit(0)                 # no opinion: normal permission handling continues
    decide("deny", f"quecto: {path} is not reserved on the board by {member}. "
                   f"Reserve it with board_reserve first. Reserved: {reserved}")
if tool == "Bash":
    cmd = inp.get("command", "")
    for bad in ("rm -rf /", "git push"):
        if bad in cmd:
            decide("deny", f"quecto: command refused by the swarm denylist ({bad!r})")
sys.exit(0)   # no opinion: fall through to normal permission handling
