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
    },
    "config": {
      "type": "string",
      "description": "Path to a config file to pass to the child via --config (optional)"
    },
    "workflow": {
      "type": "boolean",
      "description": "Start the child with --workflow (model selects a template itself)"
    },
    "workflow_spec": {
      "type": "object",
      "description": "Assign a specific workflow to the child by value: { \"template\": { full inline template } }. The child runs exactly that template, bound, in Active mode — no model-driven selection."
    }
  }
}
```

- **`task` is optional.** Omitting it creates an idle agent ready for prompts via `agent_cmd`.
- **`agent_id`** must be unique. Spawning with an already-running ID returns an error.
- Returns immediately (< 1 second) after the child's socket is ready.
- **`workflow_spec` vs `workflow`.** `workflow: true` makes the workflow tool available so the *child* picks a template; `workflow_spec` hands the child a specific template **by value** and binds it. They are independent of `config`, which supplies the child's runtime (providers/model/default template library).

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

#### Assigning a bound workflow (`workflow_spec`)

A parent can hand a child a specific workflow **by value** — the full template
travels in the spawn call, so the child does not need that template in its own
config. The assigned child starts in **Active** mode bound to exactly that
template: it cannot select a different template, and on completion it reports
its result rather than picking a new workflow.

```json
{
  "name": "spawn",
  "arguments": {
    "agent_id": "pr-reviewer",
    "task": "Review PR #682",
    "workflow_spec": {
      "template": {
        "id": "review-pr",
        "label": "Review PR",
        "description": "Analyze a diff and run the test suite",
        "steps": [
          { "key": "analyze", "label": "Analyze the diff", "phase": "review" },
          { "key": "verify",  "label": "Run the test suite", "phase": "review" }
        ]
      }
    }
  }
}
```

- The `template` is the full definition (same shape as a `workflow-config.json`
  template); see the `workflow` doc (`docs {"name":"workflow"}`) for the field reference.
- The spec is size-bounded (256 KiB) and written to a private, single-use file
  the child deletes once read.
- If a spec is assigned but cannot be loaded, the child **fails closed** (it
  refuses to start a workflow rather than falling back to free selection).
- `workflow_spec` cannot be combined with the child's `--no-workflow`.

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
      "enum": ["prompt", "steer", "follow_up", "abort", "kill", "await",
               "get_state", "get_messages", "get_messages_tail",
               "get_session_stats", "get_subagents", "get_extensions",
               "set_model", "clear_history", "reload_extensions"],
      "description": "Command to send"
    },
    "message": {
      "type": "string",
      "description": "Message for prompt/steer/follow_up commands"
    },
    "count": {
      "type": "integer",
      "description": "Number of messages for get_messages_tail (default: 1)"
    },
    "timeout": {
      "type": "integer",
      "description": "Max seconds for await (default: 300)"
    },
    "idle_timeout": {
      "type": "integer",
      "description": "Seconds agent must stay idle before await returns (default: 5). Set to 0 for immediate return."
    }
  },
  "required": ["agent_id", "command"]
}
```

**Supported commands:**

| Command | Description | Requires `message` |
|---------|-------------|--------------------|
| `prompt` | Send a task/message to the subagent | Yes |
| `steer` | Interrupt and redirect the agent | Yes |
| `follow_up` | Queue a message for after the current run | Yes |
| `abort` | Cancel the agent's current run | No |
| `kill` | Terminate the subagent process (SIGTERM) | No |
| `await` | Block until the subagent reaches a terminal state | No |
| `get_state` | Check if the agent is idle or streaming | No |
| `get_messages` | Read the full message history | No |
| `get_messages_tail` | Read the last N messages (use `count`) | No |
| `get_session_stats` | Get token usage and cost | No |
| `get_subagents` | List subagents spawned by this agent | No |
| `get_extensions` | List loaded extensions | No |
| `set_model` | Change the LLM model | No |
| `clear_history` | Clear conversation history | No |
| `reload_extensions` | Hot-reload extensions | No |

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

### `await` — block until a subagent finishes

The `await` command blocks the calling tool until the target subagent reaches
a terminal state — idle, exited, or timeout. This eliminates the need for
polling loops that burn LLM tokens.

**Parameters:**

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `timeout` | integer (seconds) | 300 | Max wall-clock wait time |
| `idle_timeout` | integer (seconds) | 5 | Seconds the agent must stay idle before returning. Resets if the agent resumes streaming (e.g. auto-continue between workflow steps). Set to 0 for immediate return on first idle. |

**Return value (structured JSON):**

```json
{
  "status": "idle",
  "reason": "completed",
  "agent_id": "bookmarks-v1",
  "elapsed_ms": 47200,
  "workflow": {
    "mode": "complete",
    "steps_completed": 7,
    "steps_total": 7
  }
}
```

| Status | Reason | Description |
|--------|--------|-------------|
| `idle` | `completed` | Agent stayed idle for the full `idle_timeout` window |
| `exited` | `exit_code_0` | Process exited cleanly |
| `exited` | `exit_code_<N>` | Process exited with error code N |
| `exited` | `signal_<N>` | Process killed by signal N |
| `timeout` | `null` | Wall-clock `timeout` exceeded |
| `error` | `agent_not_found` | Agent ID not in registry |
| `error` | `connection_failed` | Socket exists but connection refused |
| `error` | `another_await_active` | Another `await` is already waiting on this agent |

**Examples:**

```json
{"name": "agent_cmd", "arguments": {"agent_id": "reviewer", "command": "await", "timeout": 600}}
```

```json
{"name": "agent_cmd", "arguments": {"agent_id": "reviewer", "command": "await", "idle_timeout": 0}}
```

**Key behaviors:**

- **Auto-continue safe:** The `idle_timeout` window correctly filters brief
  idle gaps between auto-continue workflow steps.
- **One awaiter per agent:** A second `await` on the same agent returns
  `"another_await_active"` immediately.
- **Interacts with abort/steer/kill:** `abort` and `steer` do not interrupt
  `await` — it continues waiting. `kill` causes `await` to return with
  `"exited"` status.
- **Workflow snapshot:** The `workflow` field is a read-only snapshot of
  workflow state at the moment of return (null if workflow is not enabled).

## Observing the unit tree

Every agent in a spawn hierarchy is the same quecto unit, and its
`workflow_state` events are **identity-tagged** with `agent_id` (the agent's
own id) and `parent_id` (its spawner; `null` at the root). `spawn` passes its
own id to each child automatically, so a child's events are correctly parented
without any manual flag. From these two fields alone a consumer can rebuild the
whole parent → child tree from the event stream — no per-child polling required.

Two complementary ways to observe a child's progress:

- **Pull — `get_subagents`:** each entry now carries `parent_id` and an optional
  `workflow` snapshot (`{mode, steps_completed, steps_total}`) for that child,
  kept current by the parent's per-child monitor. Good for a point-in-time view
  of every child at once.
- **Push — forwarded events:** the parent's monitor re-emits each child's
  `workflow_state` events onto the **parent's** own event stream, re-stamped
  with the child's identity. A supervisor watching one socket sees its whole
  subtree advance live. Forwarded events are rebuilt canonically (only
  `type`/`agent_id`/`parent_id`/`mode`/`progress`), so a child cannot inject
  arbitrary fields onto the parent's stream.

Prefer reading forwarded `workflow_state` events (or one `get_subagents` call)
over repeatedly polling each child with `get_state`. See the `uds-protocol`
doc (`docs {"name":"uds-protocol"}`) for the wire shape.

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

See Disabling Tools (`docs {"name":"disable-tools"}`) for details.

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

### Parallel code review with await

Spawn multiple reviewers and await their completion:

```
"Spawn three subagents for parallel review:
1. spawn(agent_id='arch-review', task='Review src/ for architecture issues')
2. spawn(agent_id='security-review', task='Review src/ for security issues')
3. spawn(agent_id='perf-review', task='Review src/ for performance issues')

Then await each:
  agent_cmd(agent_id='arch-review', command='await', timeout=600)
  agent_cmd(agent_id='security-review', command='await', timeout=600)
  agent_cmd(agent_id='perf-review', command='await', timeout=600)

Read results with agent_cmd get_messages_tail and compile a summary."
```

All three run concurrently — total time is the max of the three, not the sum.
No polling loop needed — `await` blocks efficiently until each finishes.

### Fire-and-forget with later collection

```
"Spawn agent_id='researcher' with task='Analyze the codebase and write findings to /tmp/report.md'.
Continue working on other tasks. When ready:
  agent_cmd(agent_id='researcher', command='await', idle_timeout=0)
Read the report."
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

- Disabling Tools (`docs {"name":"disable-tools"}`) — `--disable-tool spawn` to prevent subagent spawning
- UDS Protocol Reference (`docs {"name":"uds-protocol"}`) — the JSON-lines protocol used for agent communication
- Extensions (`docs {"name":"extensions"}`) — adding custom tools to the agent
