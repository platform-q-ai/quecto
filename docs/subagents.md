# Subagents

Subagents are child quecto processes spawned by the `spawn` tool during an
agent session. They run independently, each with their own LLM context and
tool access, and report their results back to the parent agent.

## Overview

When the LLM calls the `spawn` tool, quecto launches a new `quecto agent`
process as a subprocess. The child process:

- Uses the same quecto binary (`std::env::current_exe()`)
- Inherits the parent's `QUECTO_BASE_DIR` (config, credentials, sessions)
- Inherits the parent's sandbox posture (`--no-sandbox`, `--network`)
- Gets its own session (default name: `subagent`, or a custom `agent_id`)
- Has a 24-hour timeout (`86,400s`)

The parent agent blocks until the child completes, then receives a success
or failure message.

## How the LLM uses spawn

The `spawn` tool has this schema:

```json
{
  "type": "object",
  "properties": {
    "task": {
      "type": "string",
      "description": "The task description for the subagent"
    },
    "agent_id": {
      "type": "string",
      "description": "Optional agent ID for the subagent session"
    },
    "system": {
      "type": "string",
      "description": "Optional system prompt for the subagent"
    }
  },
  "required": ["task"]
}
```

### Example tool call

```json
{
  "name": "spawn",
  "arguments": {
    "task": "Review all Python files in src/ for security vulnerabilities",
    "agent_id": "security-reviewer",
    "system": "You are a security expert. Focus on injection, auth, and data exposure."
  }
}
```

This spawns:

```bash
quecto agent -m "Review all Python files in src/ for security vulnerabilities" \
  -s security-reviewer \
  --system "You are a security expert. Focus on injection, auth, and data exposure."
```

## Sessions

Each subagent gets its own session, persisted under `<base_dir>/sessions/`.

- **Default session**: `subagent` (if no `agent_id` is provided)
- **Custom session**: The `agent_id` value is used as the session name
- Sessions persist across spawns — a subsequent spawn with the same `agent_id`
  continues the same conversation

### Session name validation

Agent IDs must contain only alphanumeric characters, hyphens, and underscores.
The following are rejected:

- Path traversal attempts (`../../tmp/evil`)
- Spaces or special characters
- Empty strings

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

## Output handling

The child's stdout and stderr are piped to `/dev/null` — the parent doesn't
capture the child's raw output. Instead, the parent receives a structured
result:

| Outcome | Result |
|---------|--------|
| Success (exit code 0) | `"Subagent 'security-reviewer' completed successfully."` |
| Failure (exit code ≠ 0) | `"Subagent 'security-reviewer' failed (exit code 1)."` |
| Timeout (24h) | `"Subagent 'security-reviewer' timed out after 86400s."` |

The child's actual work (file edits, bash commands, etc.) happens in the
shared workspace. The parent can observe the results by reading files or
running commands after the child completes.

## Agent ID allowlists

The spawn tool supports an allowlist of permitted agent IDs. When configured:

- Only IDs in the allowlist can be spawned
- Requests with unlisted IDs are rejected with an error
- An empty allowlist (the default) permits any valid agent ID

Currently, the allowlist is always empty in the CLI agent, meaning any valid
agent ID is accepted. The allowlist mechanism exists for future integrations
that need to restrict which subagents can be spawned.

## Disabling subagents

To prevent the LLM from spawning subagents entirely:

```bash
quecto agent --disable-tool spawn -m "fix the bug"
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
  │     │
  │     ├── Validates agent_id format
  │     ├── Checks allowlist (if configured)
  │     ├── Builds command: quecto agent -m <task> -s <agent_id> [--no-sandbox] [--network]
  │     ├── Sets QUECTO_BASE_DIR env var
  │     └── Spawns subprocess with 24h timeout
  │
  └── Child Agent Process
        │
        ├── Loads config from QUECTO_BASE_DIR/config.json
        ├── Initializes its own tool registry
        ├── Gets its own LLM context (no shared state with parent)
        ├── Runs the task (may call tools, including spawn for grandchildren)
        └── Exits with code 0 (success) or non-zero (failure)
```

### Key design decisions

1. **Process isolation**: Each subagent is a separate OS process. There is no
   shared memory, no shared LLM context, and no shared tool state. This
   prevents cascading failures and makes cleanup deterministic.

2. **Config inheritance**: The child re-reads `config.json` from
   `QUECTO_BASE_DIR`. This means config changes between parent startup and
   child spawn are visible to the child (which may or may not be desirable).

3. **No output capture**: The child's stdout/stderr go to `/dev/null`. The
   parent only learns success/failure/timeout. The child's actual work
   persists in the filesystem (and its session file). This avoids buffering
   potentially large outputs in memory.

4. **Grandchildren**: A child agent can itself call `spawn`, creating a tree
   of processes. Each level inherits the same sandbox/network posture. The
   24-hour timeout applies independently to each process.

## Practical patterns

### Parallel code review

The parent spawns multiple reviewers for different aspects:

```
"Spawn three subagents:
1. agent_id='arch-review', task='Review src/ for architecture issues'
2. agent_id='security-review', task='Review src/ for security vulnerabilities'
3. agent_id='perf-review', task='Review src/ for performance issues'
Then read their session files to compile a summary."
```

Note: spawns are sequential (the parent blocks on each), but each subagent
works independently. For true parallelism, use a UDS client that manages
multiple agent processes.

### Divide and conquer

Large refactoring tasks can be broken into subtasks:

```
"First spawn agent_id='migrate-tests' with task='Convert all unittest.TestCase
classes in tests/ to pytest functions'. Then spawn agent_id='fix-imports' with
task='Update all imports in src/ to use the new module structure'."
```

### Specialized system prompts

Different subagents can have different expertise:

```json
{
  "name": "spawn",
  "arguments": {
    "task": "Write comprehensive tests for src/auth/",
    "agent_id": "test-writer",
    "system": "You are a test engineering expert. Write thorough tests with edge cases, mocking, and clear assertions. Use pytest."
  }
}
```

## Limitations

1. **Sequential execution**: The parent blocks while each child runs.
   Multiple spawns execute one after another, not in parallel.

2. **No result streaming**: The parent doesn't see the child's progress.
   It only knows success/failure after the child completes.

3. **No shared context**: The child has no access to the parent's conversation
   history. It starts fresh (or continues from its own session).

4. **24-hour hard timeout**: Cannot be changed from the spawn tool. Use
   `--max-time` on the parent agent to set a tighter overall deadline.

5. **No output capture**: The child's stdout/stderr are discarded. Coordination
   happens through the filesystem (the child writes files, the parent reads them).

## See also

- [Disabling Tools](disable-tools.md) — `--disable-tool spawn` to prevent subagent spawning
- [UDS Protocol Reference](uds-protocol.md) — alternative integration model for external processes
- [Extensions](extensions.md) — adding custom tools to the agent
