@wip
Feature: Coordinator Inbox Processing
  As the coordinator subagent process
  I want to poll my inbox for commands and dispatch them through the job service
  So that the main agent's coding_job requests are handled autonomously

  The coordinator inbox processor reads pending commands from the inbox,
  dispatches each to the CodingJobService, writes responses to the outbox,
  acknowledges processed commands, and periodically writes state snapshots.
  A "shutdown" command causes the processor to exit cleanly.

  # --- Single command dispatch ---

  Scenario: Processor dispatches a run command and writes response
    Given a coordinator inbox processor with a mock job service
    And a pending inbox command with action "run" and payload {"goal":"Fix bug","repo":"test","base_ref":"main"}
    When the processor ticks once
    Then the inbox outbox should contain a response for the command
    And the inbox response should have ok true
    And the inbox response body should contain "job_id"
    And the inbox command should be acknowledged

  Scenario: Processor dispatches a status command
    Given a coordinator inbox processor with a mock job service
    And the mock job service has a job "job_001" in state "running"
    And a pending inbox command with action "status" and payload {"job_id":"job_001"}
    When the processor ticks once
    Then the inbox outbox should contain a response for the command
    And the inbox response should have ok true
    And the inbox response body should contain "running"

  Scenario: Processor dispatches a list command
    Given a coordinator inbox processor with a mock job service
    And a pending inbox command with action "list" and payload {}
    When the processor ticks once
    Then the inbox outbox should contain a response for the command
    And the inbox response should have ok true

  Scenario: Processor dispatches a cancel command
    Given a coordinator inbox processor with a mock job service
    And the mock job service has a job "job_002" in state "running"
    And a pending inbox command with action "cancel" and payload {"job_id":"job_002"}
    When the processor ticks once
    Then the inbox outbox should contain a response for the command
    And the inbox response should have ok true

  # --- Error handling ---

  Scenario: Processor returns error response for unknown action
    Given a coordinator inbox processor with a mock job service
    And a pending inbox command with action "explode" and payload {}
    When the processor ticks once
    Then the inbox outbox should contain a response for the command
    And the inbox response should have ok false
    And the inbox response error should contain "unknown action"

  Scenario: Processor returns error response when service fails
    Given a coordinator inbox processor with a mock job service
    And the mock job service will fail with "not_found"
    And a pending inbox command with action "status" and payload {"job_id":"missing"}
    When the processor ticks once
    Then the inbox outbox should contain a response for the command
    And the inbox response should have ok false
    And the inbox response error should contain "not_found"

  # --- Multiple commands ---

  Scenario: Processor handles multiple commands in one tick
    Given a coordinator inbox processor with a mock job service
    And a pending inbox command with action "list" and payload {}
    And a pending inbox command with action "list" and payload {}
    When the processor ticks once
    Then the inbox outbox should contain 2 responses
    And all inbox responses should have ok true

  # --- Shutdown ---

  Scenario: Shutdown command causes processor to signal exit
    Given a coordinator inbox processor with a mock job service
    And a pending inbox command with action "shutdown" and payload {}
    When the processor ticks once
    Then the processor should signal shutdown
    And the inbox outbox should contain a response for the command
    And the inbox response should have ok true

  # --- State snapshot ---

  Scenario: Processor writes state snapshot after tick
    Given a coordinator inbox processor with a mock job service
    And a pending inbox command with action "list" and payload {}
    When the processor ticks once
    Then a state snapshot should exist
    And the state snapshot should have alive true
