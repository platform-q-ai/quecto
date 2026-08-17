# UDS Protocol Reference

Quecto's UDS (Unix Domain Socket) mode runs a persistent agent process that accepts length-prefixed JSON commands over a local socket. This is the integration point for TUIs, IDE plugins, web UIs, Telegram bots, and any external automation.

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
- **Framing:** Length-prefixed UTF-8 JSON frames (ADR-0008), with dual-mode readers that also accept legacy `\n`-delimited JSON lines during the deprecation window. Shared bound: **8 MiB** per message (`quecto-line-io::PROTOCOL_LINE_CAP_BYTES`, including the trailing newline on legacy lines)
- **Direction:** Client sends **commands**, agent emits **events**
- **Multi-client:** Multiple clients can connect simultaneously. Events are broadcast to all clients; commands from all clients merge into a single serial dispatch loop
- **Shutdown:** By default the agent exits when all clients disconnect. Pass `--persist` to keep it running.  Socket file is removed on exit
- **Security:** Socket file is created with `chmod 0600` (owner-only). Stale sockets older than 24h are reaped on startup
- **See also:** [ADR-0008](architecture-design-records/adr-0008-length-prefixed-uds-framing-and-bounded-events.md) for version negotiation and the NDJSON deprecation window, and the [protocol capability matrix](architecture/protocol-capability-matrix.md) for the current compatibility/evolution map

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
3. `token` (zero or more) — incremental answer-text streaming tokens
4. `thinking` (zero or more) — display-safe model reasoning/thinking deltas, separate from answer text
5. `tool_execution_start` / `tool_execution_end` (if tools are called)
5. `turn_end` — LLM call completed, includes assistant message
6. `agent_end` — run finished; carries `messageRefs` for this run (legacy `messages` is empty after #1060)
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

Cancel the current agent run. **`abort` is a full stop**, not a pause: the agent
stops completely and does **not** resume on its own. If the agent is idle, this is
a no-op.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"abort"` | yes | |
| `id` | string | no | Correlation ID |

**Behavior:**
- **Agent running:** Fires the cancellation signal. The agent stops after the
  current tool completes, in-flight tool calls and their child processes (e.g. a
  long `bash`) are terminated via the process group. The cancelled run's user
  message remains in history so the next prompt sees the same interrupted turn
  the client displayed; any assistant/tool output appended after that user
  message is discarded. `abort` is not a privacy delete: clients should not use
  it to remove sensitive text that was already submitted to the agent/model.
- **Workflow auto-continue is suppressed:** if the agent is bound to a workflow,
  `abort` clears any queued work and prevents the workflow auto-continue nudge
  from re-driving it. The agent stays stopped until explicitly re-driven by a
  fresh `prompt`. There is no "abort but keep going" mode — a resumable pause
  would be a separate command, never an overload of `abort`.
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

Return live session supervision state. This is the command to use while an
agent is in flight: unlike transcript inspection, its execution and message
count fields are updated during the active turn.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"get_state"` | yes | |
| `id` | string | no | Correlation ID |

**Response data:**

```json
{
  "model": "anthropic/claude-sonnet-4-6",
  "isStreaming": true,
  "sessionKey": "cli:default",
  "messageCount": 6,
  "pendingMessageCount": 0,
  "maxContextTokens": 300000,
  "effort": "low",
  "effortLevels": ["none", "low", "medium", "high", "xhigh", "max"],
  "execution": {
    "phase": "runningTool",
    "activityGeneration": 27,
    "lastActivityAt": "2026-07-27T20:14:52Z",
    "lastActivitySecondsAgo": 7,
    "currentTool": {
      "name": "bash",
      "callId": "call-abc123",
      "startedAt": "2026-07-27T20:14:49Z",
      "elapsedSeconds": 10
    },
    "tools": {"used":["bash"],"started":9,"completed":8,"failed":1},
    "progress": {
      "state":"advancing","reason":"4 tools completed in the last 120 seconds",
      "windowSeconds":120,"lastProgressSecondsAgo":7,
      "toolCallsCompleted":4,"toolCallsFailed":1
    }
  }
}
```

| Field | Type | Description |
|---|---|---|
| `model` | string | Active model (qualified `provider/model` or bare name) |
| `isStreaming` | boolean | `true` if the agent is currently processing a prompt |
| `sessionKey` | string | Session identifier for persistence |
| `messageCount` | integer | Current user-visible canonical message count, including messages committed during an active turn |
| `pendingMessageCount` | integer | Number of queued follow-up/steer messages |
| `maxContextTokens` | integer | Active model context-window limit in tokens (`0` when unknown) |
| `effort` | string \| null | Effective session effort (`null` means provider default / unset) |
| `effortLevels` | string[] | Effort vocabulary valid for the active model's provider |
| `execution` | object | Live phase, activity watermark/timestamps, current tool, run totals, and evidence-based 120-second progress summary |
| `workflow` | object \| omitted | Workflow snapshot when workflow is enabled for the session |

`execution.phase` is `idle`, `thinking`, `runningTool`, or `finalizing`.
`currentTool` is omitted when no tool is active. Progress is based on observed
tool completions; it is not an inferred percentage of a non-workflow task.
Busy snapshot responses retain `snapshot: true`, but the execution fields and
`messageCount` are live overlays.

---

### `get_messages`

Return the stable committed conversation transcript as bounded pages (#1061).
Use this for full/end-of-turn output inspection; use `get_state` for live
in-flight supervision. While a turn is active, a busy snapshot may lag the
mutable in-flight conversation and is marked `snapshot: true`.

Omit `count` and
`before` for the newest page (up to the protocol page size of 64 messages);
pass `count: N` for the last N messages; pass `before: <messageId>` (a message
id from a prior response's `before` field) to fetch the adjacent older page.
Without an explicit `count`, history is never returned unbounded — walk
`before` cursors until `hasMoreBefore` is `false` to reach the beginning of
the session. An explicit `count` keeps the legacy last-N contract (it may
exceed one page); every response line is still byte-capped on the wire, with a
cursor advertised for anything the cap removes.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"get_messages"` | yes | |
| `id` | string | no | Correlation ID |
| `count` | integer | no | Maximum number of trailing messages to return (omit for the newest page) |
| `before` | string | no | Paging cursor: return messages strictly before this message id. An unknown/stale cursor is an error, not a silent restart |

**Response data:**

```json
{
  "messages": [
    {
      "id": "8f14e45f-ceea-4670-9f5c-2f1a72f1a72f",
      "role": "user",
      "content": "Hello",
      "toolCalls": [],
      "toolCallId": null,
      "toolName": null,
      "isError": false,
      "collapsed": false
    },
    {
      "id": "9b74e45f-ceea-4670-9f5c-2f1a72f1a730",
      "role": "assistant",
      "content": "Hi! How can I help?",
      "toolCalls": [],
      "toolCallId": null,
      "toolName": null,
      "isError": false,
      "collapsed": false
    }
  ],
  "before": "8f14e45f-ceea-4670-9f5c-2f1a72f1a72f",
  "hasMoreBefore": true
}
```

Page metadata:

| Field | Type | Description |
|---|---|---|
| `before` | string \| null | Cursor for the adjacent older page (the oldest message included in this page); `null` when the beginning of history is reached |
| `hasMoreBefore` | boolean | Whether older history exists before this page. Legacy corner: an explicit `count: 0` returns an empty page reporting `hasMoreBefore: false` with no cursor (an empty window has no oldest-included message to anchor one) |

To page back to the beginning of the session (`request` = send the command,
then read events until the `response` whose `id` matches):

```python
resp = request(sock, {"type": "get_messages", "id": "page-0"})
while resp["data"]["hasMoreBefore"]:
    resp = request(sock, {
        "type": "get_messages",
        "id": "page-" + resp["data"]["before"],
        "before": resp["data"]["before"],
    })
```

Each message contains:

| Field | Type | Description |
|---|---|---|
| `id` | string | Stable message id (#1060) — usable as a `before` cursor, a `get_message` lookup key, or a `rewind_to` target |
| `role` | `"system"` \| `"user"` \| `"assistant"` \| `"tool"` | Message author |
| `content` | string | Message text (a ladder-demoted stub when `collapsed` is true) |
| `toolCalls` | array | Tool calls made by the assistant (each with `id`, `name`, `arguments`) |
| `toolCallId` | string \| null | For `tool` messages: which tool call this is a result for |
| `toolName` | string \| null | For `tool` messages: name of the tool that produced this result |
| `isError` | boolean | Whether a `tool` message carries an error result |
| `collapsed` | boolean | `true` when the context ladder demoted this message to a stub — recall the full body on demand via `get_message` with this `id` (#1061) |
| `thinking` | array | Optional assistant-only display-safe thinking blocks, omitted when absent. Text blocks use `{ "kind": "text", "text": "..." }`; redacted/private provider blocks use `{ "kind": "redacted" }` and never expose signatures, encrypted reasoning, or redacted payload bytes. `content` remains answer-only. |

---

### `get_message`

Return one stable message by id. For oversized content, clients pass byte-range
fields and walk the response cursor until `hasMoreContent` is `false`; every
ranged response is capped to fit the UDS frame limit (#1094).

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"get_message"` | yes | |
| `id` | string | no | Correlation ID |
| `messageId` | string | yes | Stable message id from `get_messages`, `turn_end.messageRefs`, or `agent_end.messageRefs` |
| `offset` | integer | no | Content byte offset to start the returned range; omit only when the caller knows the full message fits in one frame |
| `limit` | integer | no | Requested maximum content bytes for this page; the server may return fewer bytes to preserve the frame cap |
| `agent_id` | string | no | Forward the lookup to a spawned child agent |
| `toolCallId` | string | no | When set, recover that tool call's arguments instead of the message body |

**Response data:** the message fields above plus range metadata when `offset` or
`limit` is present:

| Field | Type | Description |
|---|---|---|
| `content` | string | Returned content slice for this page |
| `offset` | integer | Byte offset of `content` in the full message |
| `nextOffset` | integer | Offset to request next; equals `contentLength` on the final page |
| `contentLength` | integer | Full message content length in bytes |
| `hasMoreContent` | boolean | `true` when the client should request another page using `nextOffset` |

Example page walk:

```json
{"type":"get_message","id":"m-page-0","messageId":"...","offset":0,"limit":65536}
```

```json
{"content":"...","offset":0,"nextOffset":65536,"contentLength":131072,"hasMoreContent":true}
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
| `contextTokens` | integer | Provider-reported prompt occupancy when available, else local estimate |
| `maxContextTokens` | integer | Active model context-window limit (`0` when unknown) |

---

### `set_model`

Switch the active model at runtime. The new model takes effect on the next prompt.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"set_model"` | yes | |
| `id` | string | no | Correlation ID |
| `model` | string | option A | Qualified model name, e.g. `"anthropic/claude-sonnet-4-6"` |
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
{"type":"set_model","id":"sm-1","model":"anthropic/claude-sonnet-4-6"}
```

```json
{"type":"set_model","id":"sm-2","provider":"anthropic","modelId":"claude-sonnet-4-6"}
```

---

### `set_effort`

Switch the session reasoning-effort level at runtime (#1067). Applied to every subsequent turn. Validated against the active model's provider vocabulary (OpenAI reasoning models use `none`–`xhigh`; Anthropic 4.6 uses `low`/`medium`/`high`/`max`).

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"set_effort"` | yes | |
| `id` | string | no | Correlation ID |
| `effort` | string | yes | Effort level string |

**Response data (success):**

```json
{"effort": "high"}
```

**Error:** `success: false` with a message listing the valid levels for the active model.

**Example:**

```json
{"type":"set_effort","id":"se-1","effort":"xhigh"}
```

---

### `list_models`

Return configured and built-in models from the runtime registry (`models.json` + built-ins).

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"list_models"` | yes | |
| `id` | string | no | Correlation ID |

---

### `new_session`

Switch to a fresh user-chat session. The previous session is saved first. Rejected while the agent is streaming.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"new_session"` | yes | |
| `id` | string | no | Correlation ID |

---

### `rewind_to`

Rewind conversation history to a selected user-message boundary. Prefer stable `messageId` (from paged history). `messageIndex` is retained only for single-page conversations; beyond one history page it is rejected rather than misapplied.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"rewind_to"` | yes | |
| `id` | string | no | Correlation ID |
| `messageId` | string | preferred | Stable message id to rewind to |
| `messageIndex` | integer | legacy | Page-local index — only honoured while history fits one page |

---

### `reload`

Force a provider/model config reload (runtime registry + config watch surfaces).

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"reload"` | yes | |
| `id` | string | no | Correlation ID |

---

### `get_subagents`

Return the current list of spawned subagents and their live status (#524).

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"get_subagents"` | yes | |
| `id` | string | no | Correlation ID |

**Response data:** an array of subagent snapshots. Each entry includes compatibility `agentId` (the user-facing display label), additive `agentUuid` (hidden durable identity for this spawn), additive `displayName` (explicit display-label alias), `status` (`starting` / `idle` / `running` / `error` / `exited`), optional `lastTool` / `lastError`, `pid`, optional `socketPath`, optional `parentId`, optional `workflow`, and `readOnly` (observer spawn with write/edit disabled). Parent tools continue to accept display labels for live subagents; clients should key durable UI/API state by `agentUuid` when present and render `displayName` / `agentId`. `status:error` and `lastError` are terminal/run-level failure signals (for example `agent_error`), not recoverable child tool `isError` results.

**Example:**

```json
{"type":"get_subagents","id":"gs-1"}
```

---

### `get_tool_catalogue` / `list_tools`

Return the rich tool catalogue snapshot for control/query clients. This is the complete bundled-native plus UDS view: each entry is a `ToolCatalogueEntry` with tool identity, description/schema, source, owner, lifecycle, configured/profile/session policy placeholders, effective availability, restriction reason, and health.

`list_tools` is accepted as a wire alias and responds with command `get_tool_catalogue`.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"get_tool_catalogue"` or `"list_tools"` | yes | |
| `id` | string | no | Correlation ID |

**Response data:**

```json
{
  "tools": [
    {
      "name": "weather",
      "description": "Get current weather for a city",
      "source": "uds",
      "owner": "uds:client:1",
      "availability": "enabled",
      "lifecycle": "runtime-loadable",
      "effectiveEnabled": true,
      "health": "healthy"
    }
  ]
}
```

Returns an empty `tools` array only when no tools are registered in the process.

---

### `set_tool_policy`

Mutate the live tool-policy overlay used by subsequent model-visible tool catalogues. The command is backward compatible: omitting `operation` keeps legacy patch semantics.

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | `"set_tool_policy"` | yes | |
| `id` | string | no | Correlation ID |
| `mutations` | array | yes for patch; may be empty for replace | Listed tool policy changes. Each item identifies a tool by `toolId` (stable id, preferred) or `name`, plus `scope` (`"none"`, `"parent"`, `"child"`, or `"both"`) and optional `reason`. |
| `mode` | `"immediateIfIdle"` \| `"atNextTurnBoundary"` | no | Defaults to `"immediateIfIdle"`. Timing is unchanged by `operation`: if the agent is busy, immediate requests queue for the next boundary. |
| `operation` | `"patch"` \| `"replace"` | no | Defaults to `"patch"`. Patch changes only listed tools. Replace treats `mutations` as the complete desired profile and applies `unlistedScope` to every currently registered, unlisted tool. |
| `unlistedScope` | scope string | required when `operation` is `"replace"` | Closed-world scope for registered tools not listed in `mutations`. |

`replace` reconciliation reports public per-tool statuses for listed and unlisted current catalogue entries (`applied`, `alreadyInState`, `blockedByRestriction`, or `unknownTool`). Known entries report the resolved catalogue `name`; when the caller supplied a different identifier such as a stable `toolId`, results include `requestedIdentifier` for audit/display. Listed unknown/removed tools remain reported as `unknownTool`; stable-id-shaped identifiers are resolved only as stable ids and do not fall through to current tool names. Registered but unlisted tools are reconciled with `unlistedScope`. Restriction ceilings still prevent widening even in replace mode.

Queued reconciliation outcomes are observable through the later `tool_policy_changed` event. When the initiating command included `id`, that event includes `correlationId` with the same value so clients can correlate application-time results to the queued request.

Examples:

```json
{"type":"set_tool_policy","mutations":[{"name":"read","scope":"child"}]}
```

```json
{"type":"set_tool_policy","operation":"replace","unlistedScope":"none","mutations":[{"toolId":"tool-read","scope":"both"}]}
```

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

**Side effect:** Broadcasts `tool_catalogue_changed` to connected control/query clients with `changedTools`, `before`, `after`, and `reason`.

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

**Side effect:** Broadcasts `tool_catalogue_changed` to connected control/query clients with `changedTools`, `before`, `after`, and `reason`.

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

Events are emitted as length-prefixed JSON frames. Every connected client receives every event (broadcast model).

### `agent_start`

Emitted when the agent begins processing a prompt.

```json
{"type":"agent_start"}
```

### `agent_end`

Emitted when the agent finishes processing a prompt run. After #1060 / ADR-0008 part 2, full message bodies are **not** re-carried: `messages` is always empty and clients resolve content via `get_message` using the stable refs (or from stream tokens they already held).

```json
{"type":"agent_end","messages":[],"messageRefs":["8f14e45f-ceea-4670-9f5c-2f1a72f1a72f"]}
```

### `workflow_idle`

Emitted after the post-turn drain finds no further workflow continuation runnable. Optional `reason` distinguishes intervention-worthy exhaustion from deliberate stops so supervisors do not alert on an abort they requested:

| `reason` | Meaning |
|---|---|
| `exhausted` | Auto-continuation gave up (no-progress / nudge cap / unfinished with no nudge) |
| `explicit_abort` | Parent explicitly aborted |
| `completed` | Workflow reached a terminal state (or none bound) |

```json
{"type":"workflow_idle","reason":"exhausted"}
```

### `token`

Incremental text token from the LLM during streaming. Tokens arrive in real time as the model generates them.

```json
{"type":"token","token":"Hello"}
```

### `thinking`

Display-safe model thinking/reasoning text from the LLM during streaming. Thinking is additive protocol data and is never part of answer `token` text. Providers may also expose redacted/private thinking metadata internally; UDS only carries visible text deltas and recovered messages only carry display-safe thinking blocks/placeholders.

```json
{"type":"thinking","text":"I should compare the alternatives."}
```

### `turn_start`

A new LLM call begins (the agent may make multiple calls per prompt if tools are involved).

```json
{"type":"turn_start"}
```

### `turn_end`

An LLM call completed. After #1060 the assistant body is not re-carried on the wire: use stream tokens and/or `messageRefs` + `get_message`. Occupancy fields power the TUI context gauge.

```json
{
  "type": "turn_end",
  "message": {
    "role": "assistant",
    "content": "",
    "messageRefs": ["9b74e45f-ceea-4670-9f5c-2f1a72f1a730"],
    "usage": {"input": 150, "output": 42, "total": 192},
    "stopReason": "end_turn",
    "contextTokens": 4200,
    "maxContextTokens": 300000,
    "contentLength": 128
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

### `tool_catalogue_changed`

Broadcast when the rich tool catalogue changes after `register_tools`, `unregister_tools`, or client disconnect. Contains changed tool names, the previous catalogue snapshot, the new snapshot, and a reason.

```json
{
  "type": "tool_catalogue_changed",
  "changedTools": ["weather"],
  "before": [],
  "after": [
    {"name": "weather", "description": "Get current weather for a city", "source": "uds", "owner": "uds:client:1"}
  ],
  "reason": "register_tool"
}
```

### `subagent_notification`

Passive child-agent notification for human/UI visibility (completion, error, exit).

```json
{"type":"subagent_notification","agentId":"reviewer","sequence":3,"message":"child exited"}
```

### `subagent_state_changed`

Broadcast replacement snapshot of all spawned subagent statuses (clients do a simple replace). Entries match the `get_subagents` shape, including `readOnly`.

```json
{"type":"subagent_state_changed","subagents":[{"agentId":"reviewer","agentUuid":"f47ac10b-58cc-4372-a567-0e02b2c3d479","displayName":"reviewer","status":"idle","pid":1234,"readOnly":true}]}
```

### `subagent_messages_appended`

Emitted when a (sub)agent completes a turn, carrying stable refs for messages appended during that turn. A child emits this on its own stream; the parent's monitor may re-stamp `agent_id` and forward it so inspectors can stream child output turn-by-turn without re-carrying full bodies (#1060).

```json
{"type":"subagent_messages_appended","agent_id":"reviewer","messages":[],"messageRefs":["…"]}
```

### `error` (lagged client)

Sent when a client falls behind on the broadcast channel (buffer overflow). The client should call `get_messages` to re-sync.

```json
{"type":"error","message":"dropped 12 events — use get_messages to re-sync"}
```

---

## Error handling

- **Malformed JSON:** Returns `response` with `command: "parse_error"` and `success: false`. The `error` text preserves the detailed serde parse error in both single-client and multi-client modes; clients that previously string-matched the old generic `"invalid JSON command"` text should switch to the structured `command: "parse_error"` / `success: false` fields.
- **Unknown command type:** Returns `response` with `success: false`
- **Line/frame too long:** Oversized inbound messages are rejected against the shared **8 MiB** protocol cap (`quecto-line-io`); clients should recover large content via ranged `get_message` rather than a single oversized frame
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
  │                               │──tool_catalogue_changed─────>│
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
  │                               │──tool_catalogue_changed─────>│
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
| `--persist` | Keep agent alive after all clients disconnect |
| `--effort <level>` | Reasoning effort (`none`/`low`/`medium`/`high`/`xhigh`/`max`). Provider vocabulary still applies at request time. Overrides config and env var |
| `--workflow` | Start workflow-driven prompt injection immediately |
| `--workflow-guards` | Enable workflow bash command guards |
| `--no-workflow` | Disable workflow tool/state/prompt |
| `--parent-id <id>` | Declare this agent's parent in the unit tree (set automatically by `spawn`) |
| `--disable-tool <name>` | Disable/hide a tool and deny re-registration (repeatable) |
| `--config <path>` | Override config file path |

> **Note:** `bash` commands run natively in the workspace and can reach
> `$HOME` (e.g. `gh` credentials, `.gitconfig`). To confine command
> execution, run Quecto inside a container.

## See also

- Extensions (`docs {"name":"extensions"}`) — adding custom tools via native config or UDS registration
- Subagents (`docs {"name":"subagents"}`) — spawning child agent processes from within a session
- Workflow Automation (`docs {"name":"workflow"}`) — configurable step-by-step development process
