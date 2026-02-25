@done
Feature: Coordinator-Worker Lifecycle Integration
  As the coding coordinator
  I need to wire the full worker lifecycle: clone, launch, stream, and export
  So that coding jobs run end-to-end from submission to artifact delivery

  The coordinator drives the lifecycle by calling begin_preparation(),
  cloning the repo, launching the worker via WorkerRuntime, streaming
  events from the worker's stdout, handling worker exit, and exporting
  artifacts. This feature tests the coordinator's orchestration of these
  steps using the MockWorkerRuntime.

  Background:
    Given a coordinator with a worker runtime for lifecycle tests
    And a repo validator that accepts "org/repo" at ref "main"

  # --- Happy path: full lifecycle ---

  Scenario: Coordinator runs a job through full lifecycle
    When a coding job is submitted for "org/repo" at "main" with goal "fix tests"
    And the coordinator begins preparation for the job
    And the repo clone succeeds with duration 1500ms
    And the coordinator marks the job ready with worker PID
    Then the lifecycle job should be in state "running"
    When the worker emits a lifecycle status event "running"
    And the worker emits a lifecycle status event "completed"
    And the lifecycle worker exits with status 0
    And the coordinator marks the lifecycle job succeeded with summary "all tests pass"
    Then the lifecycle job should be in state "succeeded"

  Scenario: Coordinator transitions job through all preparation states
    When a coding job is submitted for "org/repo" at "main" with goal "refactor"
    Then the lifecycle job should be in state "queued"
    When the coordinator begins preparation for the job
    Then the lifecycle job should be in state "preparing"
    And a lifecycle event with type "job.start" should exist

  Scenario: Coordinator records worker PID when marking ready
    When a coding job is submitted for "org/repo" at "main" with goal "add feature"
    And the coordinator begins preparation for the job
    And the repo clone succeeds with duration 200ms
    And the coordinator marks the job ready with worker PID
    Then the lifecycle job should have a worker PID set
    And a lifecycle event with type "job.ready" should exist

  # --- Worker event streaming ---

  Scenario: Coordinator receives and records worker progress events
    When a coding job is submitted for "org/repo" at "main" with goal "update docs"
    And the coordinator begins preparation for the job
    And the repo clone succeeds with duration 100ms
    And the coordinator marks the job ready with worker PID
    And the coordinator records worker progress "editing files" with completion 50
    Then the lifecycle job summary should contain "editing files"

  Scenario: Coordinator receives worker events via the runtime
    When a coding job is submitted for "org/repo" at "main" with goal "cleanup"
    And the coordinator begins preparation for the job
    And the repo clone succeeds with duration 100ms
    And the coordinator marks the job ready with worker PID
    And the worker emits a lifecycle tool.start event for "worker_edit"
    And the worker emits a lifecycle tool.result event for "worker_edit"
    Then the lifecycle events should include "tool.start" and "tool.result"

  # --- Worker exit handling ---

  Scenario: Coordinator handles successful worker exit
    When a coding job is submitted for "org/repo" at "main" with goal "fix bug"
    And the coordinator begins preparation for the job
    And the repo clone succeeds with duration 100ms
    And the coordinator marks the job ready with worker PID
    And the lifecycle worker exits with status 0
    And the coordinator marks the lifecycle job succeeded with summary "bug fixed"
    Then the lifecycle job should be in state "succeeded"
    And a lifecycle event with type "job.end" should exist

  Scenario: Coordinator handles failed worker exit
    When a coding job is submitted for "org/repo" at "main" with goal "fix compile"
    And the coordinator begins preparation for the job
    And the repo clone succeeds with duration 100ms
    And the coordinator marks the job ready with worker PID
    And the lifecycle worker exits with status 1
    And the coordinator marks the lifecycle job failed with code "internal"
    Then the lifecycle job should be in state "failed"
    And a lifecycle event with type "job.end" should exist

  # --- Clone failure ---

  Scenario: Coordinator handles clone failure during preparation
    When a coding job is submitted for "org/repo" at "main" with goal "migrate"
    And the coordinator begins preparation for the job
    And the repo clone fails with error "permission denied"
    And the coordinator marks the lifecycle job failed with code "internal"
    Then the lifecycle job should be in state "failed"

  # --- Worker timeout ---

  Scenario: Coordinator kills worker on wall timeout
    When a coding job is submitted for "org/repo" at "main" with goal "long task"
    And the coordinator begins preparation for the job
    And the repo clone succeeds with duration 100ms
    And the coordinator marks the job ready with worker PID
    And the coordinator kills the lifecycle worker due to timeout
    And the coordinator marks the lifecycle job failed with code "timeout"
    Then the lifecycle worker should not be alive
    And the lifecycle job should be in state "failed"

  # --- Cleanup ---

  Scenario: Coordinator cleans up worker resources after completion
    When a coding job is submitted for "org/repo" at "main" with goal "refactor"
    And the coordinator begins preparation for the job
    And the repo clone succeeds with duration 100ms
    And the coordinator marks the job ready with worker PID
    And the lifecycle worker exits with status 0
    And the coordinator marks the lifecycle job succeeded with summary "done"
    And the coordinator cleans up the lifecycle worker
    Then the lifecycle worker should not be alive

  # --- Cancel during run ---

  Scenario: Coordinator cancels running job and kills worker
    When a coding job is submitted for "org/repo" at "main" with goal "optimize"
    And the coordinator begins preparation for the job
    And the repo clone succeeds with duration 100ms
    And the coordinator marks the job ready with worker PID
    And the coordinator cancels the lifecycle job with reason "user_request"
    Then the lifecycle job should be in state "canceled"
    And the lifecycle worker should not be alive

  # --- Multiple jobs ---

  Scenario: Coordinator manages multiple concurrent jobs
    When a coding job is submitted for "org/repo" at "main" with goal "task A"
    And a second coding job is submitted for "org/repo" at "main" with goal "task B"
    And the coordinator begins preparation for both lifecycle jobs
    And both lifecycle repo clones succeed
    And the coordinator marks both lifecycle jobs ready
    Then 2 lifecycle workers should be running

  # --- Event count ---

  Scenario: Coordinator emits correct event sequence for full lifecycle
    When a coding job is submitted for "org/repo" at "main" with goal "full cycle"
    And the coordinator begins preparation for the job
    And the repo clone succeeds with duration 500ms
    And the coordinator marks the job ready with worker PID
    And the lifecycle worker exits with status 0
    And the coordinator marks the lifecycle job succeeded with summary "complete"
    Then the lifecycle event count should be at least 3

  # --- Status query after lifecycle ---

  Scenario: Coordinator returns status with artifacts after success
    When a coding job is submitted for "org/repo" at "main" with goal "deliver"
    And the coordinator begins preparation for the job
    And the repo clone succeeds with duration 100ms
    And the coordinator marks the job ready with worker PID
    And the lifecycle worker exits with status 0
    And the coordinator marks the lifecycle job succeeded with summary "delivered" and artifacts "patch.diff,summary.json"
    Then the lifecycle job status should include artifacts "patch.diff" and "summary.json"

  Scenario: Coordinator returns status with error details after failure
    When a coding job is submitted for "org/repo" at "main" with goal "broken"
    And the coordinator begins preparation for the job
    And the repo clone succeeds with duration 100ms
    And the coordinator marks the job ready with worker PID
    And the lifecycle worker exits with status 2
    And the coordinator marks the lifecycle job failed with code "internal" and detail "segfault in worker"
    Then the lifecycle job status should include error_code "internal"
    And the lifecycle job status should include error_detail "segfault in worker"
