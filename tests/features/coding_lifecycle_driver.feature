@done
Feature: Coding Lifecycle Driver
  As the coding runtime
  I need a lifecycle driver that progresses jobs through their states
  So that queued jobs are automatically prepared, launched, monitored, and completed

  The lifecycle driver is a tick-based orchestrator. On each tick it:
  1. Transitions queued jobs to preparing
  2. Clones repos and launches workers for preparing jobs
  3. Polls running workers for events and forwards them
  4. Detects worker exits and marks jobs succeeded or failed
  5. Handles cancellation by killing workers

  # --- Happy path ---

  Scenario: Driver transitions a queued job to preparing
    Given a lifecycle driver with a mock runtime and mirror
    And a queued coding job with goal "fix bug"
    When the driver ticks once
    Then the job should be in "preparing" state

  Scenario: Driver launches a worker after preparation
    Given a lifecycle driver with a mock runtime and mirror
    And a queued coding job with goal "add feature"
    When the driver ticks twice
    Then the job should be in "running" state
    And the job should have a worker PID assigned

  Scenario: Driver marks a job succeeded on worker exit 0
    Given a lifecycle driver with a mock runtime and mirror
    And a queued coding job with goal "refactor"
    When the driver ticks twice
    And the mock worker exits with status 0
    And the driver ticks once
    Then the job should be in "succeeded" state

  Scenario: Driver marks a job failed on worker exit 1
    Given a lifecycle driver with a mock runtime and mirror
    And a queued coding job with goal "broken"
    When the driver ticks twice
    And the mock worker exits with status 1
    And the driver ticks once
    Then the job should be in "failed" state

  # --- Event forwarding ---

  Scenario: Driver forwards worker events to the coordinator
    Given a lifecycle driver with a mock runtime and mirror
    And a queued coding job with goal "events test"
    When the driver ticks twice
    And the mock worker emits a "log.message" event
    And the driver ticks once
    Then the coordinator should have received the worker event

  # --- Clone failure ---

  Scenario: Driver marks job failed when clone fails
    Given a lifecycle driver with a failing mirror
    And a queued coding job with goal "clone fail"
    When the driver ticks twice
    Then the job should be in "failed" state
    And the job error should contain "clone"

  # --- Worker launch failure ---

  Scenario: Driver marks job failed when worker launch fails
    Given a lifecycle driver with a failing runtime
    And a queued coding job with goal "launch fail"
    When the driver ticks twice
    Then the job should be in "failed" state
    And the job error should contain "launch"

  # --- Cancellation ---

  Scenario: Driver kills worker when job is canceled
    Given a lifecycle driver with a mock runtime and mirror
    And a queued coding job with goal "cancel me"
    When the driver ticks twice
    And the job is canceled
    And the driver ticks once
    Then the mock worker should have been killed

  # --- Multiple jobs ---

  Scenario: Driver processes multiple jobs independently
    Given a lifecycle driver with a mock runtime and mirror
    And a queued coding job with goal "job A"
    And a queued coding job with goal "job B"
    When the driver ticks twice
    Then both jobs should be in "running" state
