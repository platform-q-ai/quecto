# Spike #2264: Claude Code as a swarm worker member

Disposable spike code. It has no tests and is not for delivery.

The shim runs a single long-lived `claude -p` process over stream-json. A stand-in JSON board is served to it over a hand-rolled stdio MCP server, and a PreToolUse hook enforces reservations and the denylist.

| File | What it is |
|---|---|
| `shim.py` | Spawns `claude` in its own session, drives turns over stdin and captures stream-json. It is a child subreaper and sweeps leftover descendants. Scenarios: `roundtrip`, `guards`, `midturn`, `kill`. |
| `board.py` | Stand-in board: tasks, claim tokens, reservations, submissions, inbox and outbox, stored in a flock-guarded JSON file. |
| `board_mcp.py` | MCP stdio server (JSON-RPC 2.0, newline-delimited). It handles `initialize`, `tools/list`, `tools/call` and `ping`. No SDK is needed. |
| `hook_pretool.py` | PreToolUse hook. It denies Edit/Write to paths the member has not reserved, and denies Bash commands matching a substring denylist. |
| `map_events.py` | Replays a captured stream and projects it onto quecto sub-agent protocol shapes: state, messages with ordinals, report, stats and end. |
| `samples/*.stream.jsonl` | Trimmed real captures from claude 2.1.280 with haiku: round trip, guard refusals, mid-turn wake and kill. |

## Run

```sh
mkdir -p /tmp/w && python3 shim.py /tmp/w roundtrip   # or: guards | midturn | kill (kill waits for SIGTERM to the shim)
python3 map_events.py /tmp/w/stream.jsonl
```

Environment knobs:

- `SPIKE_MODEL` (default `haiku`)
- `SPIKE_PERM` (default `bypassPermissions`)
- `SPIKE_BUDGET` (default `0.50`, passed as `--max-budget-usd`)
- `SPIKE_SETTING_SOURCES` (default `project`)
- `SPIKE_KILLSIG=KILL` to hard-kill the claude process group
- `SPIKE_SUBREAPER=0` to show orphaned Bash children surviving

The claude flags used are in `shim.py` (`cmd = [...]`).
