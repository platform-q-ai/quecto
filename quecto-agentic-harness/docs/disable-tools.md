# Disabling Tools

The `--disable-tool` flag removes specific tools from the agent's registry
before the session starts. The model never sees disabled tools in its
definitions and cannot invoke them.

## Usage

```bash
quecto agent --disable-tool <name> -m "your prompt"
```

The flag is **repeatable** — pass it multiple times to disable several tools:

```bash
quecto agent --disable-tool bash --disable-tool web_fetch -m "review this code"
```

It works in both one-shot and UDS (daemon) modes:

```bash
quecto agent --mode uds --disable-tool bash --disable-tool web_fetch
```

## Available tool names

The following core tools can be disabled:

| Tool | Name | What it does |
|------|------|-------------|
| Bash | `bash` | Execute shell commands |
| Read | `read` | Read file contents |
| Write | `write` | Create or overwrite files |
| Edit | `edit` | Surgical find-and-replace edits |
| List | `ls` | List directory contents |
| Grep | `grep` | Search file contents with regex |
| Find | `find` | Find files by name pattern |
| Web Fetch | `web_fetch` | Fetch URL contents |
| Web Search | `web_search` | Search the web via Brave API |
| Recall | `recall` | Retrieve spilled context |
| Spawn | `spawn` | Launch subagent processes |
| Workflow | `workflow` | BDD/TDD step tracking and guards |

Extension tools (registered via the extension system) can also be disabled
by their registered name.

## Warning on unknown names

If you pass a tool name that doesn't exist in the registry, quecto prints a
warning to stderr but continues normally:

```
$ quecto agent --disable-tool nonexistent -m "hello"
WARNING: --disable-tool: no tool named 'nonexistent' in the registry
```

This is intentional — a typo shouldn't block the agent from starting.

## Use cases

### Read-only code review

Disable all write-capable tools so the agent can only read and analyse:

```bash
quecto agent \
  --disable-tool bash \
  --disable-tool write \
  --disable-tool edit \
  -m "Review the code in src/ for security issues"
```

### Air-gapped environments

Prevent the agent from making any network requests:

```bash
quecto agent \
  --disable-tool web_fetch \
  --disable-tool web_search \
  --disable-tool bash \
  -m "Analyse the local codebase"
```

Note: disabling `bash` is important here because `bash` can also make
network requests (e.g. `curl`, `wget`). To restrict network access without
disabling `bash` entirely, run Quecto inside a container with no (or
restricted) network access.

### Restricting subagent spawning

Prevent the agent from launching subagents:

```bash
quecto agent --disable-tool spawn -m "fix the bug in main.rs"
```

## UDS daemon mode

When running as a daemon, `--disable-tool` applies for the lifetime of the
agent process. All clients connecting to the socket share the same
restricted tool set:

```bash
quecto agent --mode uds --disable-tool bash --socket /tmp/agent.sock
```

Disabled tool names are permanently blocked for the lifetime of the agent
process. If a UDS client attempts to register a tool with a disabled name
via `register_tools`, the registration is silently rejected. This prevents
bypassing the restriction at runtime.

## Disabling tools when spawning a child (`disable_tools` / `read_only`)

The same capability is available on the `spawn` tool, so a coordinator can
launch a child agent with a restricted tool set — the spawn path and this CLI
`--disable-tool` flag are two entry points to the same registry-removal
mechanism.

- `disable_tools: [<tool names>]` — remove the named tools from the child's
  registry before its session starts, so the child's model never sees them.
- `read_only: true` — a convenience that expands to `disable_tools: ["write",
  "edit"]`, i.e. the child keeps `bash`, `read`, `grep`, `find` and `agent_cmd`
  but cannot use the `write` or `edit` tools.

```json
{ "task": "Review PR #123 for security issues", "read_only": true }
```

As with `--disable-tool`, this is **not a hard sandbox**: a child can still
mutate the workspace via `bash` (e.g. `sed`, `>` redirects). It is
defense-in-depth against accidental writes, not an isolation boundary — for
stronger guarantees use a workspace/sandbox posture. See
`docs {"name":"subagents"}` for the full spawn reference and a read-only
reviewer example.

## Interaction with other flags

| Flag combination | Behaviour |
|-----------------|-----------|
| `--disable-tool bash --no-sandbox` | Both apply: no workspace restriction AND no bash |
| `--disable-tool recall` | Agent cannot retrieve spilled context. The agent loop still spills long outputs but the model cannot recall them — collapse stubs become permanent summaries. |

## Implementation details

The flag is processed after the full tool registry is built (core tools +
extensions) but before the registry is handed to the agent loop. Disabled
tools are removed via `ToolRegistryImpl::remove()`, which deletes both the
tool implementation and its definition entry. The removal is permanent for
the lifetime of the agent process.

## See also

- Getting Started (`docs {"name":"getting-started"}`) — quickstart guide for UDS agent integration
- UDS Protocol Reference (`docs {"name":"uds-protocol"}`) — full protocol specification
- Extensions (`docs {"name":"extensions"}`) — custom tools and how they interact with the denylist
- Subagents (`docs {"name":"subagents"}`) — spawning child agent processes
- Workflow Automation (`docs {"name":"workflow"}`) — structured development process
