# Feature: Append-Only Audit Log

## Problem

Quecto's session persistence is designed for conversation replay, not forensic analysis. Two properties make it unsuitable as an audit trail for long-running autonomous workflows:

1. **Late write** — the session file is written once on agent exit (`uds.rs:215`, `uds_multi.rs:194`). During a multi-hour autonomous run, there is no durable record on disk until the process terminates.

2. **Lossy after pruning** — `enforce_context_ceiling()` in `context_pruning.rs` drops the oldest non-pinned messages when the conversation exceeds `max_context_tokens`. The session file reflects the pruned state. Early workflow steps (RED→GREEN attempts, tool errors, retries) are permanently lost.

The spill system (`<base_dir>/spills/<key>.jsonl`) preserves tool *outputs* but not conversation structure — it cannot tell you what the agent decided, what it retried, or which workflow steps it skipped.

This means there is no complete, durable, machine-readable record of what happened during a workflow run. Without that record, automated post-run auditing (via a subagent or external tool) is impossible.

## Solution

An append-only JSONL event log, written by the engine (not the LLM), at every significant event during the agent loop. The log is:

- **Durable** — flushed to disk on every write, survives crashes and long sessions
- **Complete** — captures every tool call, tool result, workflow transition, LLM turn, and pruning event
- **Never pruned** — independent of context management, append-only
- **Machine-readable** — one JSON object per line, parseable by auditor subagents or external scripts
- **Engine-authored** — cannot be omitted or fabricated by the LLM

### File location

```
<base_dir>/audit/<session_key>.jsonl
```

Where `<base_dir>` is typically `~/.quecto` and `<session_key>` follows the existing sanitisation rules from `persistence/filename.rs` (colons become underscores, path traversal chars stripped).

### Lifecycle

- Created on first event when a session starts (lazy init)
- Appended to across agent restarts if the session key is reused
- Never truncated by the engine (users can delete manually or via a future `audit prune` command)

## Event Schema

Every line is a JSON object with a common envelope:

```json
{
  "ts": "2026-03-28T14:32:01.847Z",
  "session": "cli:my-feature",
  "turn": 7,
  "event": "<event_type>",
  ...event-specific fields
}
```

| Field | Type | Description |
|-------|------|-------------|
| `ts` | string | ISO 8601 UTC timestamp |
| `session` | string | Session key |
| `turn` | u32 | Agent loop turn counter |
| `event` | string | Event type discriminator |

### Event types

#### `tool_call`

Emitted when the agent loop dispatches a tool call.

```json
{
  "event": "tool_call",
  "tool": "bash",
  "call_id": "call_abc123",
  "arguments": "{\"command\":\"cargo test test_auth\"}"
}
```

| Field | Type | Description |
|-------|------|-------------|
| `tool` | string | Tool name |
| `call_id` | string | Tool call ID (correlates with `tool_result`) |
| `arguments` | string | Raw JSON arguments string as sent by the LLM |

#### `tool_result`

Emitted when a tool returns its result to the agent loop.

```json
{
  "event": "tool_result",
  "call_id": "call_abc123",
  "tool": "bash",
  "is_error": false,
  "content_tokens": 450,
  "content_preview": "running 3 tests\ntest test_auth_valid ... FAILED"
}
```

| Field | Type | Description |
|-------|------|-------------|
| `call_id` | string | Correlates with the originating `tool_call` |
| `tool` | string | Tool name (denormalised for query convenience) |
| `is_error` | bool | Whether the tool returned an error |
| `content_tokens` | usize | Estimated tokens of the result (via `estimate_tokens`) |
| `content_preview` | string | First 200 chars of the result content |

Full tool output is NOT included — it's already in the spill file. The audit log records *what happened*, the spill file records *what was returned*.

#### `llm_turn_start`

Emitted before sending the chat request to the provider.

```json
{
  "event": "llm_turn_start",
  "input_tokens_estimate": 45200,
  "message_count": 34
}
```

#### `llm_turn_end`

Emitted when the provider response is complete.

```json
{
  "event": "llm_turn_end",
  "input_tokens": 45200,
  "output_tokens": 1830,
  "stop_reason": "tool_use",
  "duration_ms": 4200
}
```

| Field | Type | Description |
|-------|------|-------------|
| `input_tokens` | usize | Provider-reported input tokens (or estimate) |
| `output_tokens` | usize | Provider-reported output tokens |
| `stop_reason` | string | `end_turn`, `tool_use`, `max_tokens`, etc. |
| `duration_ms` | u64 | Wall clock time for the LLM call |

#### `workflow_step`

Emitted when a workflow step is checked, unchecked, or skipped.

```json
{
  "event": "workflow_step",
  "action": "check",
  "step_index": 3,
  "step_key": "red",
  "step_label": "Ensure new/modified tests FAIL (RED)",
  "template_id": "feature"
}
```

#### `workflow_transition`

Emitted on template selection, reset, and completion.

```json
{
  "event": "workflow_transition",
  "from_mode": "selector",
  "to_mode": "active",
  "template_id": "feature",
  "issue": { "number": 42, "title": "Add auth endpoint" }
}
```

#### `context_pruned`

Emitted when `enforce_context_ceiling` or `collapse_old_tool_results` removes content.

```json
{
  "event": "context_pruned",
  "messages_dropped": 12,
  "tool_results_collapsed": 0,
  "tokens_before": 195000,
  "tokens_after": 142000
}
```

#### `subagent_spawned`

Emitted when the `spawn` tool creates a child agent.

```json
{
  "event": "subagent_spawned",
  "agent_id": "arch-review",
  "task_preview": "Review src/ for architecture issues",
  "system_preview": "You are a senior architect..."
}
```

#### `subagent_cmd`

Emitted when `agent_cmd` interacts with a child.

```json
{
  "event": "subagent_cmd",
  "agent_id": "arch-review",
  "command": "get_messages_tail",
  "count": 1
}
```

#### `guard_blocked`

Emitted when a workflow guard blocks a bash command.

```json
{
  "event": "guard_blocked",
  "command_preview": "git commit -m ...",
  "guard_message": "Complete RED-GREEN-REFACTOR before committing.",
  "before_step_key": "commit"
}
```

#### `error`

Emitted on tool execution errors, provider errors, or agent loop recovery events.

```json
{
  "event": "error",
  "source": "tool",
  "tool": "bash",
  "message": "Command timed out after 120s"
}
```

## Implementation

### New files

#### `src/infrastructure/persistence/audit_log.rs`

The writer. Thin wrapper around an append-mode file handle.

```rust
pub struct AuditLog {
    writer: tokio::sync::Mutex<tokio::io::BufWriter<tokio::fs::File>>,
    session_key: String,
}

impl AuditLog {
    pub async fn open(base_dir: &Path, session_key: &str) -> Result<Self, DomainError>;
    pub async fn emit(&self, turn: u32, event: AuditEvent) -> Result<(), DomainError>;
}
```

`emit()` serialises the event with envelope fields, writes one line, and flushes. The flush is critical — the log must survive crashes.

#### `src/domain/audit.rs`

The event enum. Pure domain type, no I/O.

```rust
pub enum AuditEvent {
    ToolCall { tool: String, call_id: String, arguments: String },
    ToolResult { call_id: String, tool: String, is_error: bool, content_tokens: usize, content_preview: String },
    LlmTurnStart { input_tokens_estimate: usize, message_count: usize },
    LlmTurnEnd { input_tokens: usize, output_tokens: usize, stop_reason: String, duration_ms: u64 },
    WorkflowStep { action: String, step_index: usize, step_key: String, step_label: String, template_id: String },
    WorkflowTransition { from_mode: String, to_mode: String, template_id: Option<String>, issue: Option<(u64, String)> },
    ContextPruned { messages_dropped: usize, tool_results_collapsed: usize, tokens_before: usize, tokens_after: usize },
    SubagentSpawned { agent_id: String, task_preview: String, system_preview: String },
    SubagentCmd { agent_id: String, command: String },
    GuardBlocked { command_preview: String, guard_message: String, before_step_key: String },
    Error { source: String, tool: Option<String>, message: String },
}
```

### Integration points

All changes are in existing code — adding an `audit_log.emit()` call at each event site. The `AuditLog` is passed as an `Option<Arc<AuditLog>>` so non-audit sessions have zero overhead.

| File | Location | Event |
|------|----------|-------|
| `application/agent_loop.rs` | Before tool dispatch | `ToolCall` |
| `application/agent_loop.rs` | After tool result received | `ToolResult` |
| `application/agent_loop.rs` | Before `build_chat_request` / provider call | `LlmTurnStart` |
| `application/agent_loop.rs` | After provider response complete | `LlmTurnEnd` |
| `application/agent_loop.rs` | Inside `apply_context_pruning`, after drop/collapse | `ContextPruned` |
| `domain/workflow/engine.rs` | `check()`, `uncheck()`, `skip()` methods | `WorkflowStep` |
| `domain/workflow/engine.rs` | `select_template()`, `reset()`, completion detection | `WorkflowTransition` |
| `infrastructure/tools/bash/mod.rs` | When a guard blocks execution | `GuardBlocked` |
| `application/subagent.rs` | `SpawnTool::execute()` | `SubagentSpawned` |
| `application/subagent.rs` | `AgentCmdTool::execute()` | `SubagentCmd` |
| `application/agent_loop.rs` | Error recovery paths | `Error` |

### Wiring

The `AuditLog` is created in the UDS entry points (`uds.rs`, `uds_multi.rs`) alongside the session store, and passed into `AgentLoop` construction. When `--workflow` is active, audit logging is enabled automatically (the audit log's primary consumer is the workflow auditor). A future `--audit-log` flag could enable it independently.

```rust
// In uds.rs / uds_multi.rs, during setup:
let audit_log = if workflow_enabled {
    Some(Arc::new(AuditLog::open(&base_dir, &session_key).await?))
} else {
    None
};
```

The `AgentLoop` stores `Option<Arc<AuditLog>>`. At each emit site:

```rust
if let Some(ref log) = self.audit_log {
    let _ = log.emit(current_turn, AuditEvent::ToolCall { ... }).await;
}
```

The `let _ =` is intentional — audit log write failures must not crash the agent. Log the error via `tracing::warn!` and continue.

### Workflow engine integration

The `WorkflowEngine` in `domain/workflow/engine.rs` is a pure domain type with no I/O. Rather than passing the audit log into the engine (which would violate the domain boundary), the caller in `agent_loop.rs` emits `WorkflowStep` and `WorkflowTransition` events after calling engine methods. The engine already returns enough information to populate the event fields.

## Testing

### Unit tests

- `audit_log.rs`: write events, read back JSONL, verify each line deserialises to correct event type and envelope fields
- `audit_log.rs`: verify flush-on-every-write (write, read file without closing, confirm content is present)
- `audit_log.rs`: verify lazy directory creation (`<base_dir>/audit/` created on first emit)
- `audit.rs`: serde round-trip for every `AuditEvent` variant

### Integration tests

- Full agent loop test with audit enabled: run a short workflow, verify the audit JSONL contains the expected sequence of events in order
- Verify audit log survives simulated crash (kill process, confirm log is complete up to last event)
- Verify `Option<Arc<AuditLog>>` = `None` path has no file I/O (no `audit/` directory created)

## Non-goals

- **Querying the audit log** — the log is a flat JSONL file. Any querying, filtering, or aggregation is the consumer's responsibility (auditor subagent, external script, etc.). The engine only writes.
- **Log rotation / retention** — out of scope. Users manage file size manually. A future `quecto audit prune --older-than 30d` could be added.
- **Full tool output in the log** — the content preview is capped at 200 chars. Full output lives in the spill file. The audit log records *what happened*, not *what was returned*.
- **Subagent audit logs** — each subagent is a separate process with its own session key. If audit is enabled for the parent, subagents inherit the flag and write their own `audit/<subagent_key>.jsonl`. No merging or cross-process coordination.

## Downstream use: workflow auditor

This feature enables automated post-run auditing. After a workflow cycle completes, the agent spawns an auditor subagent that reads `~/.quecto/audit/<session_key>.jsonl` and evaluates the run against a scoring rubric:

- Step compliance (were all steps executed in order?)
- Subagent dispatch (were reviewers spawned in parallel?)
- Efficiency (attempts before GREEN, token usage, retries)
- Guidance failures (agent asked user questions, ran wrong commands)

The auditor writes findings to `playbook.md` (actionable guidance improvements) and `results.tsv` (quantitative metrics). This creates a closed feedback loop: each workflow cycle produces data that improves the next cycle's guidance.

The auditor design is out of scope for this feature — it consumes the audit log but does not affect its implementation. This feature's only job is to produce the log.
