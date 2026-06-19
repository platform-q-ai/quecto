# UDS Protocol Reference

Quecto's UDS (Unix Domain Socket) mode runs a persistent agent process that accepts JSON-lines commands over a local socket. This is the integration point for TUIs, IDE plugins, web UIs, Telegram bots, and any external automation.

## Starting the agent

```bash
quecto agent --mode uds
# stderr: quecto-agent-socket: /tmp/quecto-agent-<uuid>.sock

# Keep the agent alive even when all clients disconnect
quecto agent --mode uds --persist
```

The agent prints the socket path to stderr on startup. Options:

| Flag | Description |
|---|---|
| `--mode uds` | Required. Run in UDS event-bus mode instead of one-shot |
| `--socket <path>` | Explicit socket path (max 104 bytes). Default: auto-generated in `$XDG_RUNTIME_DIR` or `$TMPDIR` |
| `--session <name>` | Named session for persistence across restarts |
| `--no-session` | Ephemeral mode — no session saved to disk |
| `--system <text>` | Inject a system prompt (not persisted in session history) |
| `--persist` | Stay alive when all clients disconnect. Default: agent exits when the last client disconnects |

## Wire format

- **Transport:** Unix domain socket (stream)
- **Framing:** One JSON object per line (`\n`-delimited), max 1 MiB per line
- **Direction:** Client sends **commands**, agent emits **events**
- **Multi-client:** Multiple clients can connect simultaneously. Events are broadcast to all clients; commands from all clients merge into a single serial dispatch loop
- **Shutdown:** By default the agent exits when all clients disconnect. Pass `--persist` to keep it running.  Socket file is removed on exit
- **Security:** Socket file is created with `chmod 0600` (owner-only). Stale sockets older than 24h are reaped on startup

## Correlation IDs

Every command accepts an optional `id` field (string). When present, the corresponding `response` event echoes it back. This lets clients match responses to requests when multiple commands are in flight.

```json
{"type":"get_state","id":"req-42"}
```
```json
{"type":"response","id":"req-42","command":"get_state","success":true,"data":{...}}
```

---

## Commands (client → agent)

### `prompt`

Send a user message to the agent. This is the primary command — it triggers an LLM call, possible tool executions, and streams results back as events.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"prompt"` | yes | |
| `id` | string | no | Correlation ID |
| `message` | string | yes | The user message |
| `streamingBehavior` | `"steer"` \| `"followUp"` | when agent is running | How to handle this prompt if the agent is already processing a previous one |

**Behavior:**
- When the agent is idle, starts a new run immediately
- When the agent is already running and `streamingBehavior` is:
  - `"steer"` — cancels the current run after the active tool, then processes this message
  - `"followUp"` — queues this message to run after the current run completes
  - omitted — returns an error: `"agent is running; provide streamingBehavior"`

**Events emitted** (in order for a successful run):

1. `agent_start` — run begins
2. `turn_start` — LLM call begins
3. `token` (zero or more) — incremental streaming tokens
4. `tool_execution_start` / `tool_execution_end` (if tools are called)
5. `turn_end` — LLM call completed, includes assistant message
6. `agent_end` — run finished, includes all messages from this run
7. `response` with `command: "prompt"` and `success: true`

On error, emits `response` with `command: "agent_error"` instead of steps 6-7.

After completion, any pending follow-up or steer messages are automatically processed (each triggering its own event sequence).

**Example:**

```json
{"type":"prompt","id":"p-1","message":"What files are in the current directory?"}
```

---

### `steer`

Interrupt the current agent run and deliver a new message. If the agent is idle, the message is queued for the next prompt.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"steer"` | yes | |
| `id` | string | no | Correlation ID |
| `message` | string | yes | The new instruction |

**Behavior:**
- **Agent running:** Fires the cancellation signal (interrupts after the current tool completes), then prepends this message to the pending queue so it runs next
- **Agent idle:** Queues the message. It will execute after the next `prompt` completes

**Response:** Always `success: true` (the steer is acknowledged, not a guarantee the in-flight run was cancelled — it may have already finished).

**Example:**

```json
{"type":"steer","id":"s-1","message":"Actually, focus on Python files only"}
```

---

### `follow_up`

Queue a message that will be processed after the current (or next) agent run completes. Unlike `steer`, this does not interrupt the running agent.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"follow_up"` | yes | |
| `id` | string | no | Correlation ID |
| `message` | string | yes | The follow-up message |

**Behavior:**
- **Agent running:** Appends the message to the pending queue. When the current run completes (success, error, or cancellation), pending messages are drained and executed in order.
- **Agent idle:** Appends the message to the pending queue and immediately starts draining it, matching Pi-style follow-up delivery.
- Each pending message triggers its own full event sequence (`agent_start` → `agent_end`)

**Response:** Always `success: true`.

**Example:**

```json
{"type":"follow_up","id":"fu-1","message":"Now summarize what you found"}
```

---

### `abort`

Cancel the current agent run. If the agent is idle, this is a no-op.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"abort"` | yes | |
| `id` | string | no | Correlation ID |

**Behavior:**
- **Agent running:** Fires the cancellation signal. The agent stops after the current tool completes. The user message from the cancelled run is removed from history
- **Agent idle:** No-op (acknowledged successfully)

**Response:** Always `success: true`.

**Example:**

```json
{"type":"abort","id":"ab-1"}
```

---

### `set_workflow_automation`

Toggle core workflow automation for this UDS session. Requires workflow mode.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"set_workflow_automation"` | yes | |
| `id` | string | no | Correlation ID |
| `autoContinue` | boolean | no | Enable/disable core auto-continue nudges |
| `completionNudge` | boolean | no | Enable/disable core completion nudges |

**Response data:**

```json
{"autoContinue": true, "completionNudge": true}
```

---

### `clear_history`

Clear the conversation history in-place without restarting the agent. The system prompt is preserved; all user, assistant, and tool messages are removed. Any pending follow-up/steer messages are drained. The context spill store is also cleared so that stale tool output summaries are not re-injected on the next prompt.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"clear_history"` | yes | |
| `id` | string | no | Correlation ID |

**Behavior:**
- **Agent idle:** Clears all messages except the system prompt. Drains the pending queue. Clears the spill store (index + disk file). Returns `success: true`
- **Agent running:** Returns `success: false` with error `"cannot clear history while agent is running"`

The system prompt (injected via `--system` flag) is preserved at `messages[0]`. Context-pruning manifests (`is_manifest = true`) and spill indices are **not** preserved — `recall("list")` returns empty after clear.

**Response:**

```json
{"type":"response","id":"ch-1","command":"clear_history","success":true}
```

**Error (agent is streaming):**

```json
{"type":"response","id":"ch-1","command":"clear_history","success":false,"error":"cannot clear history while agent is running"}
```

**Example:**

```json
{"type":"clear_history","id":"ch-1"}
```

---

### `list_sessions`

Return persisted CLI sessions that can be resumed by this UDS agent.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"list_sessions"` | yes | |
| `id` | string | no | Correlation ID |

**Response data:**

```json
{
  "sessions": [
    {"key":"cli:default","name":"default","messageCount":42,"updatedUnixSecs":1765930000}
  ]
}
```

Only `cli:*` sessions are returned because `resume_session` resumes CLI session names.

**Example:**

```json
{"type":"list_sessions","id":"ls-1"}
```

---

### `resume_session`

Switch the active UDS conversation to a persisted CLI session. The current session is saved first.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"resume_session"` | yes | |
| `id` | string | no | Correlation ID |
| `session` | string | yes | CLI session name, e.g. `default` or `work` |

**Behavior:**
- **Agent idle:** Saves the current session, loads `cli:<session>`, and switches subsequent prompts to that history
- **Agent running:** Returns `success: false` with error `"cannot resume a session while agent is running"`
- Invalid session names are rejected using the same rules as `quecto agent --session`
- Missing sessions return `success: false` with `"session not found: <name>"`

**Example:**

```json
{"type":"resume_session","id":"rs-1","session":"work"}
```

---

### `get_state`

Return the current session state.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"get_state"` | yes | |
| `id` | string | no | Correlation ID |

**Response data:**

```json
{
  "model": "anthropic/claude-sonnet-4-20250514",
  "isStreaming": false,
  "sessionKey": "cli:default",
  "messageCount": 6,
  "pendingMessageCount": 0
}
```

| Field | Type | Description |
|---|---|---|
| `model` | string | Active model (qualified `provider/model` or bare name) |
| `isStreaming` | boolean | `true` if the agent is currently processing a prompt |
| `sessionKey` | string | Session identifier for persistence |
| `messageCount` | integer | Total messages in conversation history |
| `pendingMessageCount` | integer | Number of queued follow-up/steer messages |

---

### `get_messages`

Return the full conversation history.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"get_messages"` | yes | |
| `id` | string | no | Correlation ID |

**Response data:**

```json
{
  "messages": [
    {
      "role": "user",
      "content": "Hello",
      "toolCalls": [],
      "toolCallId": null,
      "toolName": null
    },
    {
      "role": "assistant",
      "content": "Hi! How can I help?",
      "toolCalls": [],
      "toolCallId": null,
      "toolName": null
    }
  ]
}
```

Each message contains:

| Field | Type | Description |
|---|---|---|
| `role` | `"system"` \| `"user"` \| `"assistant"` \| `"tool"` | Message author |
| `content` | string | Message text |
| `toolCalls` | array | Tool calls made by the assistant (each with `id`, `name`, `arguments`) |
| `toolCallId` | string \| null | For `tool` messages: which tool call this is a result for |
| `toolName` | string \| null | For `tool` messages: name of the tool that produced this result |

---

### `get_messages_tail`

Return the last N messages from the conversation history. Useful for rendering recent context without fetching the entire history.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"get_messages_tail"` | yes | |
| `id` | string | no | Correlation ID |
| `count` | integer | yes | Maximum number of messages to return |

**Behavior:**
- Returns the last `count` messages in chronological order
- If `count` exceeds the total history, returns all messages
- If `count` is 0, returns an empty array

**Response data:** Same format as `get_messages`.

**Example:**

```json
{"type":"get_messages_tail","id":"gmt-1","count":4}
```

---

### `get_session_stats`

Return token usage and cost statistics for the current session.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"get_session_stats"` | yes | |
| `id` | string | no | Correlation ID |

**Response data:**

```json
{
  "sessionKey": "cli:default",
  "userMessages": 3,
  "assistantMessages": 3,
  "toolCalls": 2,
  "toolResults": 2,
  "totalMessages": 10,
  "tokens": {
    "input": 0,
    "output": 0,
    "cacheRead": 0,
    "cacheWrite": 0,
    "total": 0
  },
  "cost": 0.0
}
```

| Field | Type | Description |
|---|---|---|
| `sessionKey` | string | Session identifier |
| `userMessages` | integer | Number of user messages |
| `assistantMessages` | integer | Number of assistant messages |
| `toolCalls` | integer | Number of tool calls made |
| `toolResults` | integer | Number of tool results received |
| `totalMessages` | integer | Total messages (including system, tool) |
| `tokens` | object | Token usage breakdown (input, output, cache) |
| `cost` | number | Estimated cost in USD |

> **Note:** Token counts and cost are currently zeroed — usage tracking from the LLM response is not yet threaded through to session stats.

---

### `set_model`

Switch the active model at runtime. The new model takes effect on the next prompt.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"set_model"` | yes | |
| `id` | string | no | Correlation ID |
| `model` | string | option A | Qualified model name, e.g. `"anthropic/claude-sonnet-4-20250514"` |
| `provider` | string | option B | Provider name (used with `modelId`) |
| `modelId` | string | option B | Model ID within the provider |

You must provide either `model` OR both `provider` + `modelId`. Providing neither (or empty strings) returns an error.

**Model routing:**
- **Qualified names** (`provider/model`): Routed to the matching provider. If no provider matches the prefix, prompts will fail with `"no configured provider matches model prefix 'X'"` — but the agent stays alive and you can switch to a valid model
- **Bare names** (`model`): Sent to the first configured provider, which may not support the model

**Response:** `success: true` on valid input, `success: false` with an error message on validation failure.

> **Important:** `set_model` only swaps a string — it performs no validation against the provider. Errors surface on the next `prompt`.

**Examples:**

```json
{"type":"set_model","id":"sm-1","model":"anthropic/claude-sonnet-4-20250514"}
```

```json
{"type":"set_model","id":"sm-2","provider":"anthropic","modelId":"claude-sonnet-4-20250514"}
```

---

### `get_extensions`

Return the list of registered extensions. Includes both native extensions (from config) and UDS-registered extensions (from connected clients). Only extensions whose tools were successfully registered (not shadowing core tools) are included.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"get_extensions"` | yes | |
| `id` | string | no | Correlation ID |

**Response data:**

```json
{
  "extensions": [
    {"name": "web_search", "description": "Search the web using Brave Search or DuckDuckGo"},
    {"name": "weather", "description": "Get current weather for a city"}
  ]
}
```

Returns an empty array if no extensions are registered.

---

### `reload_extensions`

> **Deprecated.** This command exists for backward compatibility but is a no-op since v0.19.0. It returns `success: true` immediately without doing anything. Native extensions are loaded once at startup; UDS extensions are managed via `register_tools` / `unregister_tools`.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"reload_extensions"` | yes | |
| `id` | string | no | Correlation ID |

**Response:** Always `success: true`.

---

### `register_tools`

Register one or more tools from a connected extension client. See Extensions guide (`docs {"name":"extensions"}`) for full details.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"register_tools"` | yes | |
| `id` | string | no | Correlation ID |
| `tools` | array | yes | Array of tool registration objects |

Each tool object:

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | yes | Tool name (must not shadow a core tool) |
| `description` | string | yes | Description shown to the LLM |
| `parametersSchema` | string | no | JSON Schema for tool parameters. Default: `{"type":"object"}` |

**Response:**

```json
{"type":"response","id":"rt-1","command":"register_tools","success":true,"data":{"registered":["weather"]}}
```

**Side effect:** Broadcasts `extensions_changed` to all connected clients.

**Failure:** Returns `success: false` if any tool shadows a core tool name. No tools from the batch are registered.

**Idempotent:** Re-registering an existing tool updates its definition.

**Example:**

```json
{
  "type": "register_tools",
  "id": "rt-1",
  "tools": [
    {
      "name": "weather",
      "description": "Get current weather for a city",
      "parametersSchema": "{\"type\":\"object\",\"properties\":{\"city\":{\"type\":\"string\"}},\"required\":[\"city\"]}"
    }
  ]
}
```

---

### `unregister_tools`

Remove previously registered tools by name.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"unregister_tools"` | yes | |
| `id` | string | no | Correlation ID |
| `tools` | array | yes | Array of tool name strings to remove |

**Response:**

```json
{"type":"response","id":"ut-1","command":"unregister_tools","success":true,"data":{"unregistered":["weather"]}}
```

**Side effect:** Broadcasts `extensions_changed` to all connected clients.

Unknown tool names are silently ignored (not an error).

**Example:**

```json
{"type":"unregister_tools","id":"ut-1","tools":["weather"]}
```

---

### `tool_result`

Return the result of a tool execution request. Sent by an extension client in response to an `execute_tool` event.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"tool_result"` | yes | |
| `toolCallId` | string | yes | Must match the `toolCallId` from the `execute_tool` event |
| `content` | string | yes | Result text returned to the LLM |
| `isError` | boolean | no | `true` if the result represents an error. Default: `false` |

**No response event.** The result is delivered directly to the agent loop.

**Example:**

```json
{"type":"tool_result","toolCallId":"uds-0000000abc-00000001","content":"22°C, sunny","isError":false}
```

---

## Events (agent → client)

Events are emitted as JSON lines. Every connected client receives every event (broadcast model).

### `agent_start`

Emitted when the agent begins processing a prompt.

```json
{"type":"agent_start"}
```

### `agent_end`

Emitted when the agent finishes processing. Contains all messages produced during this run (assistant replies, tool calls, tool results).

```json
{"type":"agent_end","messages":[{"role":"assistant","content":"Here are the files...","toolCalls":[],"toolCallId":null,"toolName":null}]}
```

### `token`

Incremental text token from the LLM during streaming. Tokens arrive in real time as the model generates them.

```json
{"type":"token","token":"Hello"}
```

### `turn_start`

A new LLM call begins (the agent may make multiple calls per prompt if tools are involved).

```json
{"type":"turn_start"}
```

### `turn_end`

An LLM call completed. Contains the assistant's message, optional usage statistics, and tool results.

```json
{
  "type": "turn_end",
  "message": {
    "role": "assistant",
    "content": "Here are the files in the directory...",
    "usage": {"input": 150, "output": 42, "total": 192},
    "stopReason": "end_turn"
  },
  "toolResults": []
}
```

### `tool_execution_start`

A tool began executing. Includes the tool call ID (for correlation with `tool_execution_end`), tool name, and arguments.

```json
{
  "type": "tool_execution_start",
  "toolCallId": "call_abc123",
  "toolName": "bash",
  "args": {"command": "ls -la"}
}
```

### `tool_execution_end`

A tool finished executing. Includes the result and whether it was an error.

```json
{
  "type": "tool_execution_end",
  "toolCallId": "call_abc123",
  "toolName": "bash",
  "result": {"content": [{"type": "text", "text": "file1.txt\nfile2.txt"}]},
  "isError": false
}
```

### `workflow_state`

Emitted whenever an agent's workflow advances (template selected, step completed, mode change). Every `workflow_state` event is **identity-tagged** so any consumer can rebuild the unit tree from the stream alone:

```json
{
  "type": "workflow_state",
  "agent_id": "reviewer",
  "parent_id": "root",
  "mode": "active",
  "progress": {"done": 2, "total": 5}
}
```

| Field | Type | Description |
|---|---|---|
| `agent_id` | string \| null | The emitting agent's id (its session name); `null` if unnamed |
| `parent_id` | string \| null | The spawning agent's id; `null` at the root. Sourced from `--parent-id` (set automatically by `spawn`) |
| `mode` | string | Workflow mode (`selecting_template` / `active` / `complete`) |
| `progress` | object | `{done, total}` step counts (plus `percent` on the emitter's own events) |

**Forwarding (push observability).** A parent's per-child monitor re-emits each child's `workflow_state` events onto the **parent's** stream, re-stamped with the child's identity. Forwarded events are rebuilt **canonically** — only `type`, `agent_id`, `parent_id`, `mode`, and `progress` are carried; arbitrary child-supplied keys are not passed through. This lets a supervisor observe a whole subtree's progress from a single socket without polling each child (PRD Stage B).

### `response`

Direct response to a command. Carries the correlation `id` (if one was sent), the command name, success/failure, and optional data or error message.

```json
{"type":"response","id":"req-1","command":"prompt","success":true}
```

```json
{"type":"response","id":"sm-1","command":"set_model","success":false,"error":"set_model requires model, or provider+modelId"}
```

```json
{"type":"response","command":"agent_error","success":false,"error":"no configured provider matches model prefix 'gemini'"}
```

> **Note:** Agent errors from LLM failures are emitted as `response` events with `command: "agent_error"` rather than `command: "prompt"`. This distinguishes infrastructure errors from successful completions.

### `execute_tool`

Sent to the specific extension client that registered a tool when the LLM calls it. This event is **routed**, not broadcast — only the registering client receives it.

```json
{
  "type": "execute_tool",
  "toolCallId": "uds-0000000abc-00000001",
  "toolName": "weather",
  "arguments": "{\"city\":\"London\"}"
}
```

| Field | Type | Description |
|---|---|---|
| `toolCallId` | string | Unique call identifier — must be echoed back in `tool_result` |
| `toolName` | string | Name of the tool being called |
| `arguments` | string | JSON string of the tool arguments from the LLM |

The extension must respond with a `tool_result` command containing the matching `toolCallId`. If no response arrives within 30 seconds, the agent returns a timeout error to the LLM.

### `extensions_changed`

Broadcast when the extension list changes (after `register_tools`, `unregister_tools`, or client disconnect). Contains the full updated list.

```json
{
  "type": "extensions_changed",
  "extensions": [
    {"name": "web_search", "description": "Search the web using Brave Search or DuckDuckGo"},
    {"name": "weather", "description": "Get current weather for a city"}
  ]
}
```

### `error` (lagged client)

Sent when a client falls behind on the broadcast channel (buffer overflow). The client should call `get_messages` to re-sync.

```json
{"type":"error","message":"dropped 12 events — use get_messages to re-sync"}
```

---

## Error handling

- **Malformed JSON:** Returns `response` with `command: "parse_error"` and `success: false`
- **Unknown command type:** Returns `response` with `success: false`
- **Line too long:** Returns `response` with `command: "parse_error"` and error `"line exceeds 1 MiB limit"`
- **Agent error during prompt:** Returns `response` with `command: "agent_error"`. The agent stays alive — subsequent commands are processed normally
- **Unroutable model:** If `set_model` was set to a provider that doesn't exist, the next `prompt` returns an `agent_error` with `"no configured provider matches model prefix 'X'"`. Use `set_model` to switch to a valid model and retry

---

## Event sequence diagrams

### Simple prompt (no tools)

```
Client                          Agent
  │                               │
  │──prompt──────────────────────>│
  │                               │
  │<──────────────agent_start─────│
  │<──────────────turn_start──────│
  │<──────────────token───────────│  (repeated)
  │<──────────────turn_end────────│
  │<──────────────agent_end───────│
  │<──────────────response────────│  command:"prompt", success:true
```

### Prompt with tool call

```
Client                          Agent
  │                               │
  │──prompt──────────────────────>│
  │                               │
  │<──────────────agent_start─────│
  │<──────────────turn_start──────│
  │<──────────────token───────────│
  │<────────tool_execution_start──│  toolName:"bash"
  │<────────tool_execution_end────│  result, isError
  │<──────────────turn_end────────│  (LLM processes tool result)
  │<──────────────agent_end───────│
  │<──────────────response────────│  command:"prompt", success:true
```

### Prompt with follow-up

```
Client                          Agent
  │                               │
  │──follow_up───────────────────>│
  │<──────────────response────────│  command:"follow_up", success:true
  │                               │
  │──prompt──────────────────────>│
  │                               │
  │<──────────────agent_start─────│  (prompt run)
  │<──────────────...─────────────│
  │<──────────────agent_end───────│
  │<──────────────response────────│  command:"prompt", success:true
  │                               │
  │<──────────────agent_start─────│  (follow-up run — automatic)
  │<──────────────...─────────────│
  │<──────────────agent_end───────│
```

### Abort during prompt

```
Client                          Agent
  │                               │
  │──prompt──────────────────────>│
  │<──────────────agent_start─────│
  │<──────────────turn_start──────│
  │                               │
  │──abort───────────────────────>│
  │<──────────────response────────│  command:"abort", success:true
  │                               │  (agent cancelled, no agent_end)
```

### Error and recovery

```
Client                          Agent
  │                               │
  │──set_model("gemini/pro")─────>│
  │<──────────────response────────│  command:"set_model", success:true
  │                               │
  │──prompt──────────────────────>│
  │<──────────────agent_start─────│
  │<──────────────turn_start──────│
  │<──────────────response────────│  command:"agent_error", error:"no configured provider..."
  │                               │
  │──set_model("anthropic/...")──>│
  │<──────────────response────────│  command:"set_model", success:true
  │                               │
  │──prompt──────────────────────>│  (succeeds now)
  │<──────────────agent_start─────│
  │<──────────────...─────────────│
  │<──────────────agent_end───────│
  │<──────────────response────────│  command:"prompt", success:true
```

### Extension registration and tool execution

```
Extension Client                Agent                    Other Clients
  │                               │                          │
  │──register_tools──────────────>│                          │
  │<──────────────response────────│  success:true            │
  │                               │──extensions_changed─────>│
  │                               │                          │
  │              ... LLM calls the registered tool ...       │
  │                               │                          │
  │<──────────execute_tool────────│  (routed, not broadcast) │
  │                               │                          │
  │──tool_result─────────────────>│                          │
  │                               │                          │
  │  ... tool_execution_start/end broadcast to all ...       │
  │<────tool_execution_end────────│──tool_execution_end─────>│
```

### Extension disconnect cleanup

```
Extension Client                Agent                    Other Clients
  │                               │                          │
  │──[disconnect]────────────────>│                          │
  │                               │  (auto-unregister tools) │
  │                               │──extensions_changed─────>│
```

---

## Connecting with common tools

### socat

```bash
socat - UNIX-CONNECT:/tmp/quecto-agent-<uuid>.sock
```

### Python

```python
import socket, json

sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
sock.connect("/tmp/quecto-agent-<uuid>.sock")

def send(cmd):
    sock.sendall((json.dumps(cmd) + "\n").encode())

def recv_lines():
    buf = b""
    while True:
        data = sock.recv(4096)
        if not data:
            break
        buf += data
        while b"\n" in buf:
            line, buf = buf.split(b"\n", 1)
            yield json.loads(line)

send({"type": "prompt", "id": "p1", "message": "Hello!"})
for event in recv_lines():
    print(event["type"], event.get("command", ""))
    if event.get("type") == "response" and event.get("command") == "prompt":
        break

sock.close()
```

### Node.js

```javascript
const net = require('net');
const readline = require('readline');

const sock = net.createConnection('/tmp/quecto-agent-<uuid>.sock');
const rl = readline.createInterface({ input: sock });

rl.on('line', (line) => {
  const event = JSON.parse(line);
  console.log(event.type, event.command || '');
});

sock.write(JSON.stringify({type: 'prompt', id: 'p1', message: 'Hello!'}) + '\n');
```

---

## Startup flags reference

All flags for `quecto agent` that affect UDS mode:

| Flag | Description |
|------|-------------|
| `--mode uds` | Required. Run in UDS mode |
| `--socket <path>` | Explicit socket path (max 104 bytes). Default: auto in `$XDG_RUNTIME_DIR` or `$TMPDIR` |
| `-s` / `--session <name>` | Named session for persistence. Default: `cli:default` |
| `--no-session` | Ephemeral mode — no session saved/loaded |
| `--system <text>` | System prompt (not persisted in session) |
| `--model <model>` | Override default model from config |
| `--max-iterations <n>` | Max tool call rounds per prompt |
| `--max-time <secs>` | Wall-clock timeout for the entire agent |
| `--no-sandbox` | Disable workspace path restriction (DANGEROUS) |
| `--persist` | Keep agent alive after all clients disconnect |
| `--effort <level>` | Effort level for 4.6 models (`low`/`medium`/`high`/`max`). Overrides config and env var |
| `--disable-tool <name>` | Remove a tool from the registry (repeatable) |
| `--config <path>` | Override config file path |

> **Note:** `bash` commands run natively in the workspace and can reach
> `$HOME` (e.g. `gh` credentials, `.gitconfig`). To confine command
> execution, run Quecto inside a container.

## See also

- Extensions (`docs {"name":"extensions"}`) — adding custom tools via native config or UDS registration
- Subagents (`docs {"name":"subagents"}`) — spawning child agent processes from within a session
- Workflow Automation (`docs {"name":"workflow"}`) — configurable step-by-step development process
- Disabling Tools (`docs {"name":"disable-tools"}`) — restricting which tools the agent can access
