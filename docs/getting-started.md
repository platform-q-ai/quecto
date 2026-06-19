# Getting Started with the UDS Agent

This guide walks through setting up and using the quecto UDS agent — the
integration point for TUIs, IDE plugins, web UIs, and automation scripts.

## Prerequisites

1. **quecto installed**: `cargo install --path .` or download a release binary
2. **Config file**: Run `quecto onboard` to create `~/.quecto/config.json`
3. **API key**: At least one provider (OpenAI or Anthropic) configured

## Starting the agent

### Auto-generated socket (recommended)

```bash
quecto agent --mode uds
```

The socket path is printed to stderr:

```
quecto-agent-socket: /run/user/1000/quecto-agent-a1b2c3d4-e5f6-7890-abcd-ef1234567890.sock
```

### Explicit socket path

```bash
quecto agent --mode uds --socket /tmp/my-agent.sock
```

### Persistent mode

By default, the agent exits when the last client disconnects. Use
`--persist` to keep it running:

```bash
quecto agent --mode uds --persist
```

Shutdown via `SIGTERM` or `SIGINT` (Ctrl+C).

### With session persistence

```bash
quecto agent --mode uds -s my-project
```

The session is saved to `~/.quecto/sessions/` and restored on restart
with the same session name.

### Using `quecto-tui`

If you prefer a terminal UI instead of working with `socat` directly, run the
workspace member:

```bash
# Conversational TUI; workflow tool is available but dormant
cargo run -p quecto-tui --

# Workflow-driven TUI; model is prompted to enter workflow mode immediately
cargo run -p quecto-tui -- --workflow --workflow-guards
```

By default, `quecto-tui` spawns `quecto agent --mode uds` automatically and
connects to the announced socket. You can also attach to an existing agent:

```bash
cargo run -p quecto-tui -- --socket /tmp/agent.sock
```

When `quecto-tui` spawns the agent for you, it can also forward several useful
agent flags:

- default launch exposes the workflow tool but does not prompt the model to start a workflow
- `--workflow` starts workflow-driven prompt injection immediately
- `--workflow-guards` enables workflow bash guards; it does not by itself force workflow prompt injection
- `--no-workflow` hides the workflow tool/state/prompt entirely
- `--system <prompt>` passes a custom system prompt through to the spawned agent
- `--config <path>` uses an alternate quecto config file
- `--no-sandbox` disables filesystem sandboxing for the spawned agent

`bash` commands run natively as your user — the workspace is just the working
directory, **not** a confinement boundary. Commands can read your home directory
(`~/.ssh`, cloud/`git` credentials) and reach the network, so tools like
`gh auth status` and `git push` work without extra configuration. The command
denylist is a best-effort speed-bump, not a security boundary, and there are no
process/resource limits. To run untrusted input safely, run Quecto in a
container that is non-root, with minimal/read-only mounts, cgroup limits
(`--memory`/`--pids-limit`/`--cpus`), and a restricted network. See the
[Security section in the README](../README.md#security).

For safety, auto-discovered socket paths are validated and must live under
`/tmp`, `$TMPDIR`, `$XDG_RUNTIME_DIR`, or `$HOME`.

Useful built-in shortcuts and commands:

- `Enter` sends the current message
- `Shift+Enter` inserts a newline
- `Ctrl+L` opens the model selector
- `Ctrl+O` toggles tool output expansion
- `Ctrl+Shift+A` toggles workflow auto-continue
- `Ctrl+Shift+N` toggles workflow completion nudge
- `/model`, `/clear`, `/new`, `/session`, `/workflow-auto`, `/workflow-nudge`, `/help`, `/quit`

## Your first prompt

Connect with `socat` and send a prompt:

```bash
# Terminal 1: start the agent
quecto agent --mode uds --socket /tmp/agent.sock

# Terminal 2: connect and send a prompt
echo '{"type":"prompt","id":"p1","message":"Hello!"}' | socat - UNIX-CONNECT:/tmp/agent.sock
```

You'll receive a stream of JSON events:

```json
{"type":"agent_start"}
{"type":"turn_start"}
{"type":"token","token":"Hello"}
{"type":"token","token":"!"}
{"type":"token","token":" How"}
{"type":"token","token":" can"}
{"type":"token","token":" I"}
{"type":"token","token":" help"}
{"type":"token","token":"?"}
{"type":"turn_end","message":{"role":"assistant","content":"Hello! How can I help?",...}}
{"type":"agent_end","messages":[{"role":"assistant","content":"Hello! How can I help?",...}]}
{"type":"response","id":"p1","command":"prompt","success":true}
```

## Common patterns

### Interactive client (Python)

```python
import socket, json, threading

def connect(path):
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sock.connect(path)
    return sock

def send(sock, cmd):
    sock.sendall((json.dumps(cmd) + "\n").encode())

def listen(sock, callback):
    buf = b""
    while True:
        data = sock.recv(4096)
        if not data:
            break
        buf += data
        while b"\n" in buf:
            line, buf = buf.split(b"\n", 1)
            callback(json.loads(line))

# Usage
sock = connect("/tmp/agent.sock")

def on_event(event):
    if event["type"] == "token":
        print(event["token"], end="", flush=True)
    elif event["type"] == "agent_end":
        print()  # newline after tokens
    elif event["type"] == "response" and not event.get("success", True):
        print(f"Error: {event.get('error', 'unknown')}")

# Start listener in background
thread = threading.Thread(target=listen, args=(sock, on_event), daemon=True)
thread.start()

# Send prompts
send(sock, {"type": "prompt", "id": "p1", "message": "List files in current directory"})
```

### Interrupt and redirect

```python
# While the agent is processing "analyze all files":
send(sock, {"type": "steer", "id": "s1", "message": "Focus only on .py files"})
```

The agent cancels after the current tool completes, then processes the
steer message.

### Queue follow-up work

```python
# Queue work before or during a prompt:
send(sock, {"type": "follow_up", "id": "fu1", "message": "Now write tests for what you found"})
send(sock, {"type": "prompt", "id": "p1", "message": "Review src/auth.py for bugs"})
# After p1 completes, fu1 runs automatically
```

### Switch models mid-session

```python
send(sock, {"type": "set_model", "id": "sm1", "model": "anthropic/claude-sonnet-4-20250514"})
# Next prompt uses the new model
send(sock, {"type": "prompt", "id": "p2", "message": "Explain this code"})
```

### Check session state

```python
send(sock, {"type": "get_state", "id": "gs1"})
# Response: {"type":"response","id":"gs1","command":"get_state","success":true,
#   "data":{"model":"...","isStreaming":false,"messageCount":6,...}}

send(sock, {"type": "get_session_stats", "id": "st1"})
# Response includes token usage, message counts, estimated cost
```

### Clear conversation history

```python
send(sock, {"type": "clear_history", "id": "ch1"})
# Clears all messages except system prompt; fails if agent is streaming
```

## Multi-client architecture

Multiple clients can connect to the same socket simultaneously:

```
┌─────────┐
│ TUI     │──┐
└─────────┘  │
┌─────────┐  │     ┌──────────────┐
│ IDE     │──┼────>│ quecto agent │
└─────────┘  │     │ (UDS mode)   │
┌─────────┐  │     └──────────────┘
│ Script  │──┘
└─────────┘
```

- **Events are broadcast**: All clients receive all events
- **Commands are serialized**: Commands from all clients merge into a single
  dispatch loop (no concurrent session mutation)
- **Max 64 clients**: Additional connections are accepted but may receive
  lagged-event warnings
- **Lagged clients**: If a client can't keep up with events, it receives
  an error event and should call `get_messages` to re-sync

## Restricting tools

Use `--disable-tool` to remove tools before the agent starts:

```bash
# Read-only mode: no file writes or shell commands
quecto agent --mode uds \
  --disable-tool bash \
  --disable-tool write \
  --disable-tool edit

# Air-gapped: no network access
quecto agent --mode uds \
  --disable-tool web_fetch \
  --disable-tool web_search \
  --disable-tool bash

# No subagent spawning
quecto agent --mode uds --disable-tool spawn
```

Disabled tools are permanently blocked — even UDS `register_tools` cannot
re-add them. See [Disabling Tools](disable-tools.md).

## Adding custom tools via extensions

External processes can register tools at runtime:

```json
{
  "type": "register_tools",
  "id": "rt-1",
  "tools": [{
    "name": "weather",
    "description": "Get weather for a city",
    "parametersSchema": "{\"type\":\"object\",\"properties\":{\"city\":{\"type\":\"string\"}},\"required\":[\"city\"]}"
  }]
}
```

When the LLM calls the tool, only the registering client receives the
`execute_tool` event. The client responds with `tool_result`.

See [Extensions](extensions.md) for the full guide.

## Error recovery

The agent stays alive after errors. Common patterns:

### Provider error → switch model

```python
# Prompt fails with "no configured provider matches model prefix 'gemini'"
send(sock, {"type": "set_model", "model": "anthropic/claude-sonnet-4-20250514"})
send(sock, {"type": "prompt", "message": "Try again"})
```

### Abort a stuck run

```python
send(sock, {"type": "abort"})
# Agent stops after current tool, ready for next prompt
```

### Re-sync after lag

```python
# Received: {"type":"error","message":"dropped 12 events — use get_messages to re-sync"}
send(sock, {"type": "get_messages"})
# Full history returned
```

## Lifecycle management

### Starting

```bash
# Background the agent
quecto agent --mode uds --persist --socket /tmp/agent.sock &
AGENT_PID=$!
echo "Agent PID: $AGENT_PID, socket: /tmp/agent.sock"
```

### Health check

```bash
echo '{"type":"get_state"}' | socat -t2 - UNIX-CONNECT:/tmp/agent.sock
```

### Graceful shutdown

```bash
kill $AGENT_PID  # SIGTERM
# Agent cleans up socket file and exits
```

### Stale socket cleanup

Quecto automatically reaps stale socket files older than 24 hours on
startup. If an agent was killed with `SIGKILL`, the socket file may
remain — it will be cleaned up the next time any quecto agent starts.

## See also

- [UDS Protocol Reference](uds-protocol.md) — complete command and event specification
- [Extensions](extensions.md) — adding custom tools
- [Subagents](subagents.md) — spawning child agent processes
- [Workflow Automation](workflow.md) — structured development process
- [Disabling Tools](disable-tools.md) — restricting agent capabilities
