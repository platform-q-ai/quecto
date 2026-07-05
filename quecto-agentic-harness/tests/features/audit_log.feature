@done
Feature: Append-only audit log

  An append-only JSONL event log written by the engine at every significant
  event during the agent loop. The log is durable (flushed on every write),
  complete (captures tool calls, LLM turns, workflow transitions, pruning, etc.),
  never pruned, and machine-readable.

  Background:
    Given a temporary audit log directory

  # --- Domain: AuditEvent serde ---

  Scenario: AuditEvent ToolCall round-trips through JSON
    Given an AuditEvent::ToolCall with tool "bash" call_id "call_1" arguments "{\"command\":\"ls\"}"
    When the event is serialized to JSON
    Then it deserializes back to an identical [ToolCall] event

  Scenario: AuditEvent ToolResult round-trips through JSON
    Given an AuditEvent::ToolResult with call_id "call_1" tool "bash" is_error false content_tokens 450 content_preview "ok"
    When the event is serialized to JSON
    Then it deserializes back to an identical [ToolResult] event

  Scenario: AuditEvent LlmTurnStart round-trips through JSON
    Given an [AuditEvent]::LlmTurnStart with input_tokens_estimate 45200 message_count 34
    When the event is serialized to JSON
    Then it deserializes back to an identical LlmTurnStart event

  Scenario: AuditEvent LlmTurnEnd round-trips through JSON
    Given an [AuditEvent]::LlmTurnEnd with input_tokens 45200 output_tokens 1830 stop_reason "tool_use" duration_ms 4200
    When the event is serialized to JSON
    Then it deserializes back to an identical LlmTurnEnd event

  Scenario: AuditEvent WorkflowStep round-trips through JSON
    Given an [AuditEvent]::WorkflowStep with action "check" step_index 3 step_key "red" step_label "Ensure tests fail" template_id "feature"
    When the event is serialized to JSON
    Then it deserializes back to an identical WorkflowStep event

  Scenario: AuditEvent WorkflowTransition round-trips through JSON
    Given an [AuditEvent]::WorkflowTransition from "selector" to "active" template_id "feature" issue 42 "Add auth"
    When the event is serialized to JSON
    Then it deserializes back to an identical WorkflowTransition event

  Scenario: AuditEvent ContextPruned round-trips through JSON
    Given an [AuditEvent]::ContextPruned with messages_dropped 12 tool_results_collapsed 0 tokens_before 195000 tokens_after 142000
    When the event is serialized to JSON
    Then it deserializes back to an identical ContextPruned event

  Scenario: AuditEvent SubagentSpawned round-trips through JSON
    Given an [AuditEvent]::SubagentSpawned with agent_id "arch-review" task_preview "Review src/" system_preview "You are..."
    When the event is serialized to JSON
    Then it deserializes back to an identical SubagentSpawned event

  Scenario: AuditEvent SubagentCmd round-trips through JSON
    Given an [AuditEvent]::SubagentCmd with agent_id "arch-review" command "get_messages_tail"
    When the event is serialized to JSON
    Then it deserializes back to an identical SubagentCmd event

  Scenario: AuditEvent SubagentCmd redacts a secret API key in the command
    Given a redacting [AuditEvent]::SubagentCmd for agent_id "arch-review" command "deploy --api-key=sk-abc123SECRETvalue stack"
    When the event is serialized to JSON
    Then the serialized JSON does not contain "sk-abc123SECRETvalue"
    And the serialized JSON contains "[REDACTED]"
    And the serialized JSON contains "deploy"
    And the serialized JSON contains "stack"

  Scenario: AuditEvent ProviderError round-trips through JSON
    Given an AuditEvent::ProviderError with provider "fireworks" class "client" http_status 400 body "{\"error\":\"bad\"}"
    When the event is serialized to JSON
    Then it deserializes back to an identical ProviderError event

  Scenario: AuditEvent ProviderError retains the full untruncated body
    Given a redacting AuditEvent::ProviderError for provider "fireworks" class "client" http_status 400 with a 5000 char body containing secret "sk-abc123SECRETvalue"
    When the event is serialized to JSON
    Then the serialized JSON does not contain "sk-abc123SECRETvalue"
    And the serialized JSON contains "[REDACTED]"
    And the persisted ProviderError body is at least 4000 characters

  # --- Behaviour: terminal provider failure is persisted by the agent loop (#937) ---

  Scenario: A terminal provider failure persists the full redacted error to the audit log
    Given a provider that fails terminally with a 5000 char body containing secret "sk-abc123SECRETvalue"
    When the agent processes a turn against that provider
    Then the audit log contains exactly one provider error event
    And the persisted provider error body is the full untruncated text
    And the persisted provider error body has the secret redacted

  Scenario: AuditEvent GuardBlocked round-trips through JSON
    Given an [AuditEvent]::GuardBlocked with command_preview "git commit" guard_message "Complete steps first" before_step_key "commit"
    When the event is serialized to JSON
    Then it deserializes back to an identical GuardBlocked event

  Scenario: AuditEvent Error round-trips through JSON
    Given an AuditEvent::Error with source "tool" tool "bash" message "Command timed out"
    When the event is serialized to JSON
    Then it deserializes back to an identical Error event

  # --- Infrastructure: AuditLog writer ---

  Scenario: AuditLog creates audit directory lazily on first emit
    Given no audit directory exists
    When an AuditLog is opened for [session] "test-session"
    And a [ToolCall] event is emitted at turn 1
    Then the audit directory exists
    And the file "test-session.jsonl" exists in the audit directory

  Scenario: AuditLog writes valid JSONL with envelope fields
    When an AuditLog is opened for [session] "cli:my-feature"
    And a [ToolCall] event is emitted at turn 7 with tool "bash" call_id "call_abc" arguments "{\"command\":\"test\"}"
    Then the audit file contains exactly 1 lines
    And line 1 has field "ts" matching ISO 8601
    And line 1 has field "session" equal to "cli:my-feature"
    And line 1 has field "turn" equal to 7
    And line 1 has field "event" equal to "tool_call"
    And line 1 has field "tool" equal to "bash"

  Scenario: AuditLog appends multiple events in order
    When an AuditLog is opened for [session] "multi-event"
    And a [ToolCall] event is emitted at turn 1
    And a [ToolResult] event is emitted at turn 1
    And a LlmTurnStart event is emitted at turn 2
    And a LlmTurnEnd event is emitted at turn 2
    Then the audit file contains exactly 4 lines
    And line 1 has field "event" equal to "tool_call"
    And line 2 has field "event" equal to "tool_result"
    And line 3 has field "event" equal to "llm_turn_start"
    And line 4 has field "event" equal to "llm_turn_end"

  Scenario: AuditLog flushes on every write (survives without close)
    When an AuditLog is opened for [session] "flush-test"
    And a [ToolCall] event is emitted at turn 1
    Then the audit file is readable without closing the log
    And it contains 1 complete JSON lines

  Scenario: AuditLog uses sanitized session key for filename
    When an AuditLog is opened for [session] "cli:my-feature"
    Then the audit file is named "cli_my-feature.jsonl"

  Scenario: None audit log produces no file I/O
    Given no audit log is configured
    When the disabled audit path is exercised
    Then no audit directory is created

  Scenario: Content preview in ToolResult is capped at 200 chars
    Given a tool result with 500 characters of content
    When the content_preview is generated for the audit event
    Then the content_preview is at most 200 characters
