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
