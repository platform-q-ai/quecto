#!/usr/bin/env python3
"""Hand-rolled MCP stdio server (JSON-RPC 2.0, newline-delimited) exposing the stand-in board.
Spawned by `claude` from --mcp-config; env SPIKE_BOARD, SPIKE_MEMBER, SPIKE_CWD. Disposable."""
import json, os, sys, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import board

MEMBER = os.environ.get("SPIKE_MEMBER", "W1")
CWD = os.environ.get("SPIKE_CWD", os.getcwd())
LOG = os.environ.get("SPIKE_MCP_LOG")

def S(props, req=()):
    return {"type": "object", "properties": props, "required": list(req)}
STR = {"type": "string"}
TOOLS = {
  "board_summary": ("Show swarm board tasks, their state, and your unread message count.", S({}),
                    lambda a: board.summary(MEMBER)),
  "board_claim": ("Claim a ready task. Returns a claim token needed for reserve/submit.", S({"task_id": STR}, ["task_id"]),
                  lambda a: board.claim(MEMBER, a["task_id"])),
  "board_reserve": ("Reserve files (paths relative to your working dir) you will edit for a claimed task. Edits to unreserved files are refused.",
                    S({"task_id": STR, "token": STR, "files": {"type": "array", "items": STR}}, ["task_id", "token", "files"]),
                    lambda a: board.reserve(a["task_id"], a["token"], a["files"], CWD)),
  "board_submit": ("Submit a claimed task with evidence (the command you ran and its output).",
                   S({"task_id": STR, "token": STR, "evidence": STR}, ["task_id", "token", "evidence"]),
                   lambda a: board.submit(a["task_id"], a["token"], a["evidence"])),
  "board_inbox": ("Read your unread board messages.", S({}), lambda a: board.inbox(MEMBER)),
  "board_send": ("Send a board message to another member or 'coordinator'.", S({"to": STR, "text": STR}, ["to", "text"]),
                 lambda a: board.send(MEMBER, a["to"], a["text"])),
  "board_ack": ("Acknowledge (mark read) a board message by id.", S({"id": {"type": "integer"}}, ["id"]),
                lambda a: board.ack(MEMBER, int(a["id"]))),
}

def log(obj):
    if LOG:
        with open(LOG, "a") as f: f.write(json.dumps({"t": time.time(), **obj}) + "\n")

def reply(mid, result=None, error=None):
    msg = {"jsonrpc": "2.0", "id": mid}
    if error: msg["error"] = error
    else: msg["result"] = result
    sys.stdout.write(json.dumps(msg) + "\n"); sys.stdout.flush()

for line in sys.stdin:
    if not line.strip(): continue
    req = json.loads(line); log({"in": req})
    method, mid = req.get("method"), req.get("id")
    if method == "initialize":
        reply(mid, {"protocolVersion": req["params"].get("protocolVersion", "2025-06-18"),
                    "capabilities": {"tools": {}}, "serverInfo": {"name": "quecto-board-spike", "version": "0.0.1"}})
    elif method == "tools/list":
        reply(mid, {"tools": [{"name": n, "description": d, "inputSchema": s} for n, (d, s, _) in TOOLS.items()]})
    elif method == "tools/call":
        name, args = req["params"]["name"], req["params"].get("arguments") or {}
        if name in TOOLS:
            try:
                out = TOOLS[name][2](args); is_err = "error" in out
            except Exception as e:
                out, is_err = {"error": repr(e)}, True
            reply(mid, {"content": [{"type": "text", "text": json.dumps(out)}], "isError": is_err})
        else:
            reply(mid, error={"code": -32602, "message": f"unknown tool {name}"})
    elif method == "ping":
        reply(mid, {})
    elif mid is not None:
        reply(mid, error={"code": -32601, "message": f"method not found: {method}"})
    # notifications (no id) are ignored
