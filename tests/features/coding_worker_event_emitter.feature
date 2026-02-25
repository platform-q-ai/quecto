@done
Feature: Worker Event Emitter
  As the nsjail coding worker process
  I need to emit structured events as JSON Lines to stdout
  So that the host coordinator can stream and persist worker progress

  The worker runs inside nsjail and communicates with the coordinator
  via JSON Lines on stdout. The event emitter builds EventEnvelopes
  with proper versioning, timestamps, sequence numbers, and payloads,
  then serializes them as single-line JSON to a Write sink.

  Background:
    Given a worker event emitter for run "run-1" and job "job-1"

  # --- Basic emission ---

  Scenario: Emitter produces valid JSON Lines for a tool.start event
    When the worker emits a "tool.start" event with payload:
      | tool    | worker_edit |
      | call_id | call-001    |
    Then the emitted line should be valid JSON
    And the emitted event should have version matching "1.0"
    And the emitted event should have source "worker"
    And the emitted event should have run_id "run-1"
    And the emitted event should have job_id "job-1"
    And the emitted event should have type "tool.start"
    And the emitted event should have seq 1

  Scenario: Emitter increments sequence numbers
    When the worker emits a "tool.start" event with payload:
      | tool    | worker_edit |
      | call_id | call-001    |
    And the worker emits a "tool.result" event with payload:
      | tool    | worker_edit |
      | call_id | call-001    |
      | ok      | true        |
    Then the last emitted event should have seq 2

  Scenario: Emitter produces a log.message event
    When the worker emits a "log.message" event with payload:
      | level   | info                   |
      | message | starting code analysis |
    Then the emitted event should have type "log.message"
    And the emitted event payload should have "level" equal to "info"

  Scenario: Emitter produces a job.status event
    When the worker emits a "job.status" event with payload:
      | state   | running          |
      | summary | editing files    |
    Then the emitted event should have type "job.status"
    And the emitted event payload should have "summary" equal to "editing files"

  # --- Payload handling ---

  Scenario: Emitter includes all payload fields in the output
    When the worker emits a "tool.result" event with payload:
      | tool        | worker_grep |
      | call_id     | call-002    |
      | ok          | true        |
      | duration_ms | 42          |
    Then the emitted event payload should have "tool" equal to "worker_grep"
    And the emitted event payload should have "duration_ms" equal to "42"

  Scenario: Emitter sets timestamp in ISO 8601 format
    When the worker emits a "log.message" event with payload:
      | level   | debug        |
      | message | test message |
    Then the emitted event timestamp should match ISO 8601 format

  # --- Multiple events ---

  Scenario: Emitter writes each event as a separate line
    When the worker emits 3 "log.message" events
    Then 3 lines should have been emitted
    And each emitted line should be valid JSON

  Scenario: Emitter handles rapid successive emissions
    When the worker emits 10 "log.message" events
    Then 10 lines should have been emitted
    And the last emitted event should have seq 10

  # --- Error handling ---

  Scenario: Emitter rejects unknown event types gracefully
    When the worker tries to emit an event with type "unknown.bad_type"
    Then the emission should return an error
    And the emission error should mention "unknown event type"

  # --- Envelope validation ---

  Scenario: Emitter uses contract version 1.0
    When the worker emits a "job.status" event with payload:
      | state   | running      |
      | summary | working      |
    Then the emitted event should have version matching "1.0"

  Scenario: Emitter sets source to worker for all events
    When the worker emits a "tool.start" event with payload:
      | tool    | worker_read |
      | call_id | call-003    |
    Then the emitted event should have source "worker"

  # --- Write sink ---

  Scenario: Emitter writes to a provided Write sink
    Given a worker event emitter writing to a buffer
    When the worker emits a "log.message" event with payload:
      | level   | info       |
      | message | buffered   |
    Then the buffer should contain exactly one JSON line

  Scenario: Emitted lines end with a newline character
    When the worker emits a "log.message" event with payload:
      | level   | info |
      | message | test |
    Then the raw emitted output should end with a newline
