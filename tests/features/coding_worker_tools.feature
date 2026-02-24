@pending
Feature: Coding Worker Tool Execution Events
  As the coding runtime coordinator
  I want to receive structured tool execution events from workers
  So that I can track progress, build audit trails, and report to the main agent

  Workers emit tool.start and tool.result events for every tool call.
  The coordinator persists these in the JSONL event log and uses them
  to build progress summaries and artifact references.

  Background:
    Given a coding coordinator with a mock worker
    And a coding job in state "running"

  # --- Tool start/result events ---

  Scenario: Worker emits tool.start before execution
    When the worker begins executing tool "read_file" with call_id "c1"
    Then a "tool.start" event should be emitted
    And the payload should include tool "read_file" and call_id "c1"

  Scenario: Worker emits tool.result after successful execution
    When the worker completes tool "read_file" with call_id "c1" successfully
    Then a "tool.result" event should be emitted with ok true
    And the payload should include duration_ms

  Scenario: Worker emits tool.result after failed execution
    When the worker fails tool "exec" with call_id "c2"
    Then a "tool.result" event should be emitted with ok false
    And the payload should include stderr_ref pointing to an artifact

  Scenario: Tool result includes diff reference for edit operations
    When the worker completes tool "edit_file" with call_id "c3" successfully
    Then the tool.result payload should include diff_ref

  Scenario: Tool result indicates truncation for oversized output
    When the worker produces tool output exceeding the capture limit
    Then the tool.result payload should have truncated set to true
    And the full output should be spilled to an artifact

  # --- Tool start includes args preview ---

  Scenario: Tool start includes a preview of arguments
    When the worker begins executing tool "edit_file" with call_id "c4"
    And the arguments contain a file path "src/parser.rs"
    Then the "tool.start" event payload should include args_preview containing "src/parser.rs"

  # --- Event ordering ---

  Scenario: Tool events have correct sequence numbers
    When the worker executes tools "read_file" then "edit_file" then "exec"
    Then the tool events should have monotonically increasing seq numbers
    And each tool.start should precede its corresponding tool.result

  # --- Artifact creation events ---

  Scenario: Worker creates artifact and emits artifact.created event
    When the worker generates a patch file for its edits
    Then an "artifact.created" event should be emitted
    And the payload should include artifact_id, artifact_type "patch", and path
    And the artifact file should exist in the job artifact directory

  Scenario: Worker creates log artifact for exec output
    When the worker runs a shell command with significant output
    Then an "artifact.created" event should be emitted with artifact_type "log"

  # --- Log events ---

  Scenario: Worker emits structured log messages
    When the worker logs an info message "starting test suite"
    Then a "log.message" event should be emitted with level "info"
    And the message should be "starting test suite"

  Scenario: Worker log events include context when available
    When the worker logs a warning with context about a specific file
    Then the "log.message" payload should include the context field

  # --- stdout_ref on tool.result ---

  Scenario: Tool result includes stdout reference for exec output
    When the worker completes tool "exec" with call_id "c5" successfully
    And the command produces captured stdout
    Then the tool.result payload should include stdout_ref pointing to an artifact

  # --- Artifact type coverage ---

  Scenario: Worker creates summary artifact
    When the worker generates a job summary document
    Then an "artifact.created" event should be emitted with artifact_type "summary"

  Scenario: Worker creates test output artifact
    When the worker captures test runner output
    Then an "artifact.created" event should be emitted with artifact_type "test_output"
    And the payload should include size_bytes

  Scenario: Worker creates review artifact from child agent
    When a child agent produces a review document
    Then an "artifact.created" event should be emitted with artifact_type "review"

  Scenario: Worker creates snapshot artifact for skill content
    When the coordinator snapshots injected skills at job start
    Then an "artifact.created" event should be emitted with artifact_type "snapshot"

  # --- Edge cases ---

  Scenario: Unknown tool name in tool.start is still recorded
    When the worker begins executing an unrecognized tool "custom_lint" with call_id "c6"
    Then a "tool.start" event should be emitted with tool "custom_lint"
    And the event should be persisted in the event log

  Scenario: Concurrent tool executions have distinct call_ids
    When the worker starts two tools concurrently with call_ids "c7" and "c8"
    Then both "tool.start" events should be emitted
    And both "tool.result" events should arrive with their respective call_ids

  Scenario: Tool execution that exceeds timeout emits failure result
    When the worker executes tool "exec" with call_id "c9"
    And the tool execution exceeds the configured timeout
    Then a "tool.result" event should be emitted with ok false
    And the payload should include stderr_ref with timeout details

  Scenario: Tool event payload exceeding 1 MiB is truncated
    When the worker emits a tool.result with payload larger than 1 MiB
    Then the event should be truncated to fit the 1 MiB limit
    And the truncated field should be set to true

  # --- Log level coverage ---

  Scenario: Worker emits error-level log message
    When the worker logs an error message "compilation failed with 3 errors"
    Then a "log.message" event should be emitted with level "error"

  Scenario: Worker emits debug-level log message
    When the worker logs a debug message "entering parse_expression"
    Then a "log.message" event should be emitted with level "debug"

  # --- Artifact description field ---

  Scenario: Artifact event includes optional description
    When the worker creates an artifact with description "final patch for parser refactor"
    Then the "artifact.created" payload should include description "final patch for parser refactor"
