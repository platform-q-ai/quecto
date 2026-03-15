# Subagents

Subagents are child quecto processes spawned by the `spawn` tool during an
agent session. They run as **background UDS-mode agents** — the parent returns
immediately and interacts with children asynchronously via the `agent_cmd`
tool. No external dependencies (no `ncat`, `socat`, or `bash` intermediary).

## Overview

When the LLM calls the `spawn` tool, quecto launches a new `quecto agent`
process in UDS mode (`--mode uds --persist`). The child process:

- Uses the same quecto binary (`std::env::current_exe()`)
- Inherits the parent's `QUECTO_BASE_DIR` (config, credentials, sessions)
- Inherits the parent's sandbox posture (`--no-sandbox`, `--network`)
- Gets its own session (default name: `subagent`, or a custom `agent_id`)
- Listens on a Unix domain socket for commands
- Runs in the background — the parent is **not blocked**

The parent interacts with the child using the `agent_cmd` tool, which
connects to the child's UDS socket directly from Rust.

## Tools

### `spawn` — launch a subagent

```json
{
  "type": "object",
  "properties": {
    "task": {
      "type": "string",
      "description": "Initial task to send (optional — starts idle if omitted)"
    },
    "agent_id": {
      "type": "string",
      "description": "Session name for the subagent (used to address it via agent_cmd)"
    },
    "system": {
      "type": "string",
      "description": "System prompt for the subagent"
    }
  }
}
```

- **`task` is optional.** Omitting it creates an idle agent ready for prompts via `agent_cmd`.
- **`agent_id`** must be unique. Spawning with an already-running ID returns an error.
- Returns immediately (< 1 second) after the child's socket is ready.

**Example:**

```json
{
  "name": "spawn",
  "arguments": {
    "task": "Review all Python files in src/ for security vulnerabilities",
    "agent_id": "security-reviewer",
    "system": "You are a security expert."
  }
}
```

**Return value:**

```
Subagent 'security-reviewer' is running. Use agent_cmd to interact.
```

### `agent_cmd` — interact with a subagent

```json
{
  "type": "object",
  "properties": {
    "agent_id": {
      "type": "string",
      "description": "ID of the spawned subagent"
    },
    "command": {
      "type": "string",
      "enum": ["prompt", "get_state", "get_messages_tail", "steer", "abort", "get_session_stats"],
      "description": "Command to send"
    },
    "message": {
      "type": "string",
      "description": "Message for prompt/steer commands"
    },
    "count": {
      "type": "integer",
      "description": "Number of messages for get_messages_tail (default: 1)"
    }
  },
  "required": ["agent_id", "command"]
}
```

**Supported commands:**

| Command | Description | Requires `message` |
|---------|-------------|--------------------|
| `prompt` | Send a task/message to the subagent | Yes |
| `get_state` | Check if the agent is idle or streaming | No |
| `get_messages_tail` | Read the last N messages (use `count`) | No |
| `steer` | Interrupt and redirect the agent | Yes |
| `abort` | Cancel the agent's current run | No |
| `get_session_stats` | Get token usage and cost | No |

**Examples:**

```json
{"name": "agent_cmd", "arguments": {"agent_id": "security-reviewer", "command": "get_state"}}
```

```json
{"name": "agent_cmd", "arguments": {"agent_id": "security-reviewer", "command": "get_messages_tail", "count": 3}}
```

```json
{"name": "agent_cmd", "arguments": {"agent_id": "security-reviewer", "command": "steer", "message": "Focus on auth vulnerabilities only"}}
```

## Sessions

Each subagent gets its own session, persisted under `<base_dir>/sessions/`.

- **Default session**: `subagent` (if no `agent_id` is provided)
- **Custom session**: The `agent_id` value is used as the session name
- Sessions persist across spawns — a subsequent spawn with the same `agent_id`
  continues the same conversation (after the previous child has exited)

### Session name validation

Agent IDs must contain only alphanumeric characters, hyphens, and underscores
(`[a-zA-Z0-9_-]`, 1–64 characters). The following are rejected:

- Path traversal attempts (`../../tmp/evil`)
- Spaces or special characters
- Empty strings or strings longer than 64 characters

The same validation is applied in both `spawn` and `agent_cmd`.

## Sandbox inheritance

The child inherits the parent's security posture:

| Parent flag | Child behavior |
|------------|---------------|
| `--no-sandbox` active | Child gets `--no-sandbox` (unrestricted file access) |
| `--no-sandbox` not set | Child uses default workspace restriction from config |
| `--network` active | Child gets `--network` (bash network access enabled) |
| `--network` not set | Child uses default network isolation |

This ensures consistent security boundaries across the agent hierarchy. A
child agent cannot escalate its own privileges beyond what the parent has.

## Agent ID allowlists

The spawn tool supports an allowlist of permitted agent IDs. When configured:

- Only IDs in the allowlist can be spawned
- Requests with unlisted IDs are rejected with an error
- An empty allowlist (the default) permits any valid agent ID

Currently, the allowlist is always empty in the CLI agent, meaning any valid
agent ID is accepted. The allowlist mechanism exists for future integrations
that need to restrict which subagents can be spawned.

## Child process lifecycle

### Startup

1. `spawn` launches the child with `quecto agent --mode uds --socket <path> --persist`
2. Polls for socket readiness (100ms intervals, 10s timeout)
3. If the socket does not become ready, the child is killed and an error is returned
4. Registers the child in the shared `SubagentRegistry` (agent_id → socket path + PID)
5. If `task` was provided, sends it as the initial `prompt` via UDS (fire-and-forget)

### Running

- The child runs independently as a background process
- The parent continues its agent loop and can spawn additional children
- The parent interacts with children via `agent_cmd` (native UDS, no subprocess)
- Multiple children can run concurrently

### Cleanup

- **Background reaper**: Each child has a `tokio::spawn` reaper task that calls
  `child.wait()` and removes the registry entry when the child exits
- **Explicit shutdown**: `shutdown_all()` sends SIGTERM to all tracked children
  and clears the registry
- **Socket cleanup**: Socket files are removed by the child's UDS server on exit.
  Stale sockets older than 24h are reaped on next agent startup

### Duplicate prevention

Spawning with an `agent_id` that is already in the registry returns an error:

```
Failed to spawn subagent: agent 'worker-1' is already running
```

Wait for the existing agent to finish (check with `agent_cmd get_state`) or
`abort` it before spawning a new one with the same ID.

## Disabling subagents

To prevent the LLM from spawning subagents entirely:

```bash
quecto agent --disable-tool spawn --disable-tool agent_cmd -m "fix the bug"
```

See [Disabling Tools](disable-tools.md) for details.

## REPL subagent commands

The interactive REPL provides commands for managing subagents:

| Command | Description |
|---------|-------------|
| `/spawn <task>` | Spawn a task as a child agent |
| `/spawn --agent <id> <task>` | Spawn with a specific agent ID |
| `/spawn --system <prompt> <task>` | Spawn with a custom system prompt |
| `/spawn --model <model> <task>` | Spawn with a specific model |
| `/spawn --max-time <secs> <task>` | Spawn with a wall-clock timeout |
| `/spawn --help` | Show spawn command help |
| `/agent list` | List available agent profiles |
| `/agent create <name>` | Create a new agent profile |
| `/agent show <name>` | Show an agent profile's configuration |
| `/agent edit <name>` | Edit an agent profile |
| `/agent remove <name>` | Remove an agent profile |
| `/agent run <name> <task>` | Run a task using an agent profile |

## Architecture

```
Parent Agent Process
  │
  ├── LLM calls spawn(task="review code", agent_id="reviewer")
  │
  ├── SpawnTool::execute()
  │     ├── Validates agent_id format + allowlist
  │     ├── Rejects if agent_id already running
  │     ├── Launches: quecto agent --mode uds --socket <path> --persist
  │     ├── Polls for socket readiness (up to 10s)
  │     ├── Registers in SubagentRegistry (agent_id → socket_path + PID)
  │     ├── Sends initial task as UDS prompt (if provided)
  │     ├── Spawns background reaper task
  │     └── Returns immediately: "Subagent 'reviewer' is running."
  │
  ├── LLM calls agent_cmd(agent_id="reviewer", command="get_state")
  │
  ├── AgentCmdTool::execute()
  │     ├── Validates agent_id format
  │     ├── Looks up socket path in SubagentRegistry
  │     ├── Connects to UDS socket
  │     ├── Sends JSON command, reads response (300s timeout)
  │     └── Returns structured response to LLM
  │
  └── Child Agent Process (reviewer)
        ├── Listening on /run/user/1000/quecto-agent-reviewer.sock
        ├── Loads config from QUECTO_BASE_DIR/config.json
        ├── Has its own LLM context (no shared state with parent)
        ├── Processes prompts via UDS protocol
        └── Exits when no more work / on SIGTERM
```

### Key design decisions

1. **Non-blocking spawn**: The parent returns in < 1 second. Three spawns
   that each take 60s cost ~3s setup time, then all run concurrently.

2. **Native UDS interaction**: `agent_cmd` connects to child sockets directly
   from Rust. No `ncat`, `socat`, or `bash` subprocess. Works in sandboxed
   environments where external tools may not be available.

3. **Shared registry**: `SubagentRegistry` (`Arc<Mutex<HashMap>>`) maps
   agent_id to socket path + PID. Shared between `spawn` and `agent_cmd`
   via `Arc`. Entries are auto-removed when children exit.

4. **Process isolation**: Each subagent is a separate OS process. There is no
   shared memory, no shared LLM context, and no shared tool state.

5. **Config inheritance**: The child re-reads `config.json` from
   `QUECTO_BASE_DIR`. Config changes between parent startup and child spawn
   are visible to the child.

6. **Grandchildren**: A child agent can itself call `spawn`, creating a tree
   of processes. Each level inherits the same sandbox/network posture.

## Practical patterns

### Parallel code review

Spawn multiple reviewers and poll for results:

```
"Spawn three subagents for parallel review:
1. spawn(agent_id='arch-review', task='Review src/ for architecture issues')
2. spawn(agent_id='security-review', task='Review src/ for security issues')
3. spawn(agent_id='perf-review', task='Review src/ for performance issues')

Then poll each with agent_cmd get_state until all are idle,
read results with agent_cmd get_messages_tail, and compile a summary."
```

All three run concurrently — total time is the max of the three, not the sum.

### Fire-and-forget with later collection

```
"Spawn agent_id='researcher' with task='Analyze the codebase and write findings to /tmp/report.md'.
Continue working on other tasks. Periodically check:
  agent_cmd(agent_id='researcher', command='get_state')
When it's idle, read the report."
```

### Idle agents with on-demand prompts

```
"Spawn agent_id='helper' with no task (starts idle).
Later, when I need help:
  agent_cmd(agent_id='helper', command='prompt', message='Explain this function...')
  agent_cmd(agent_id='helper', command='get_messages_tail', count=1)"
```

### Steering a running agent

```
"The security reviewer is taking too long on low-priority files.
  agent_cmd(agent_id='security-review', command='steer', message='Skip test files, focus on src/auth/ only')
"
```

### Aborting a stuck agent

```
"The researcher seems stuck.
  agent_cmd(agent_id='researcher', command='abort')
"
```

## See also

- [Disabling Tools](disable-tools.md) — `--disable-tool spawn` to prevent subagent spawning
- [UDS Protocol Reference](uds-protocol.md) — the JSON-lines protocol used for agent communication
- [Extensions](extensions.md) — adding custom tools to the agent
