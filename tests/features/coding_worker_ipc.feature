@done
Feature: Worker IPC Integration
  As the coding runtime
  I need the worker entrypoint to wire the event emitter as a WorkerEventSink
  So that cmd_worker drives the agent loop and emits JSON Lines via IPC

  The WorkerEventSinkAdapter bridges the infrastructure-layer
  WorkerEventEmitter (which needs &mut self) to the domain-layer
  WorkerEventSink trait (which uses &self) by wrapping the emitter in
  a Mutex. The cmd_worker function builds all components and calls
  run_worker_loop, producing structured JSON Lines on its output writer.

  # --- WorkerEventSinkAdapter ---

  Scenario: Adapter implements WorkerEventSink via Mutex wrapper
    Given a sink adapter wrapping a buffer emitter for run "r1" and job "j1"
    When I emit a "log.message" event through the adapter
    Then the adapter emit should succeed with a sequence number

  Scenario: Adapter produces valid JSON Lines with envelope fields
    Given a sink adapter wrapping a buffer emitter for run "r1" and job "j1"
    When I emit a "log.message" event through the adapter
    Then the adapter output should contain valid JSON with run_id "r1"
    And the adapter output should contain valid JSON with job_id "j1"
    And the adapter output should contain a "ts" field

  Scenario: Adapter rejects unknown event types
    Given a sink adapter wrapping a buffer emitter for run "r1" and job "j1"
    When I emit a "bad.unknown" event through the adapter
    Then the adapter emit should fail with "unknown event type"

  Scenario: Adapter assigns incrementing sequence numbers
    Given a sink adapter wrapping a buffer emitter for run "r1" and job "j1"
    When I emit 3 "log.message" events through the adapter
    Then the adapter should have assigned sequences 1, 2, 3

  # --- cmd_worker with injected provider ---

  Scenario: cmd_worker runs agent loop and emits JSON Lines
    Given a temporary worker job directory
    And an IPC mock provider that returns "task completed"
    When I run cmd_worker with the mock provider
    Then the IPC worker exit code should be 0
    And the IPC worker output should contain at least 2 JSON lines
    And the IPC worker output should include a "log.message" event

  Scenario: cmd_worker emits ready event as first lifecycle event
    Given a temporary worker job directory
    And an IPC mock provider that returns "done"
    When I run cmd_worker with the mock provider
    Then the IPC worker first JSON line should be a "log.message" event
    And the IPC worker first event message should contain "ready"

  Scenario: cmd_worker emits done event as last lifecycle event
    Given a temporary worker job directory
    And an IPC mock provider that returns "all done"
    When I run cmd_worker with the mock provider
    Then the IPC worker last event message should contain "worker done"

  Scenario: cmd_worker returns exit code 1 on provider error
    Given a temporary worker job directory
    And an IPC mock provider that returns an error "connection refused"
    When I run cmd_worker with the mock provider
    Then the IPC worker exit code should be 1
    And the IPC worker output should include a "log.message" event

  Scenario: cmd_worker still validates arguments before running
    When I run cmd_worker IPC with no arguments
    Then the IPC worker exit code should be 1
    And the IPC worker stderr should contain "missing required flag"
