@pending
Feature: Coding Job Lifecycle
  As the coding runtime coordinator
  I want to manage coding jobs through a well-defined state machine
  So that jobs are started, tracked, and cleaned up reliably

  The coordinator exposes a command API (run/status/cancel/cleanup) and
  emits JSONL events for every state transition. Jobs follow the state
  machine: queued -> preparing -> running -> succeeded/failed/canceled.
  The main agent calls the coding_job tool; all execution is async.

  # --- Run command ---

  Scenario: Start a coding job successfully
    Given a coding coordinator with a mock worker
    When the main agent requests a coding job with goal "Add unit tests for parser"
    And repo "test-repo" at base ref "main"
    Then the coordinator should return a run_id and job_id
    And the job state should be "queued"
    And a "job.start" event should be emitted with the goal and branch

  Scenario: Run command rejects invalid repo
    Given a coding coordinator with a mock worker
    When the main agent requests a coding job with repo "nonexistent-repo"
    Then the run command should fail with error code "invalid_repo"
    And no job directory should be created

  Scenario: Run command rejects invalid base ref
    Given a coding coordinator with a mock worker
    When the main agent requests a coding job with repo "test-repo" at base ref "nonexistent-branch"
    Then the run command should fail with error code "invalid_base_ref"

  Scenario: Run command rejects policy-denied job
    Given a coding coordinator with skill denylist containing "forbidden-skill"
    When the main agent requests a coding job with skills including "forbidden-skill"
    Then the run command should fail with error code "policy_denied"

  # --- State machine transitions ---

  Scenario: Job transitions from queued to preparing to running
    Given a coding coordinator with a mock worker
    And a coding job in state "queued"
    When the coordinator begins preparation
    Then the job state should transition to "preparing"
    And when the clone completes and worker starts
    Then a "job.ready" event should be emitted with the worker PID
    And the job state should transition to "running"

  Scenario: Job transitions from running to succeeded
    Given a coding coordinator with a mock worker
    And a coding job in state "running"
    When the worker completes successfully
    Then a "job.end" event should be emitted with state "succeeded"
    And the event should include a summary and artifact references

  Scenario: Job transitions from running to failed
    Given a coding coordinator with a mock worker
    And a coding job in state "running"
    When the worker fails with a tool error
    Then a "job.end" event should be emitted with state "failed"
    And the event should include error_code "tool_error" and is_retriable

  Scenario: Job transitions from running to blocked
    Given a coding coordinator with a mock worker
    And a coding job in state "running"
    When the worker needs a main-agent decision
    Then a "job.blocked" event should be emitted with the reason
    And the job state should be "blocked"

  Scenario: Blocked job resumes after main-agent decision
    Given a coding coordinator with a mock worker
    And a coding job in state "blocked"
    When the main agent provides a decision
    Then a "job.resumed" event should be emitted
    And the job state should transition to "running"

  Scenario: Queued job can transition directly to failed on validation error
    Given a coding coordinator with a mock worker
    And a coding job in state "queued"
    When validation fails before preparation begins
    Then the job state should transition to "failed"
    And the error_code should indicate the validation failure

  Scenario: Preparing job can transition to blocked on transient clone failure
    Given a coding coordinator with a mock worker
    And a coding job in state "preparing"
    When the mirror clone fails transiently
    Then the job state should transition to "blocked"
    And the reason should describe the clone failure

  # --- Cancel command ---

  Scenario: Cancel a running job
    Given a coding coordinator with a mock worker
    And a coding job in state "running"
    When the main agent cancels the job
    Then a "job.cancel" event should be emitted with reason "user_request"
    And the job state should be "canceled"

  Scenario: Cancel a queued job before execution starts
    Given a coding coordinator with a mock worker
    And a coding job in state "queued"
    When the main agent cancels the job
    Then the job state should be "canceled"
    And no worker process should have been launched

  Scenario: Job is canceled on wall timeout
    Given a coding coordinator with a mock worker
    And a coding job with max_wall_seconds 5
    When the job exceeds the wall timeout
    Then a "job.cancel" event should be emitted with reason "wall_timeout"
    And the job state should be "canceled"

  # --- Status command ---

  Scenario: Query status of a running job
    Given a coding coordinator with a mock worker
    And a coding job in state "running" with progress 60
    When the main agent queries job status
    Then the response should include state "running" and progress 60
    And the response should include the current todo list

  Scenario: Query status of a failed job includes error details
    Given a coding coordinator with a mock worker
    And a coding job in state "failed" with error_code "timeout"
    When the main agent queries job status
    Then the response should include error_code "timeout" and error_detail

  Scenario: Query status by run_id
    Given a coding coordinator with a mock worker
    And a coding job with a known run_id
    When the main agent queries status by run_id
    Then the response should include the job state and summary

  # --- Cleanup command ---

  Scenario: Cleanup a succeeded job removes directory
    Given a coding coordinator with a mock worker
    And a coding job in state "succeeded"
    When the main agent requests cleanup
    Then the job directory should be removed
    And the response should indicate cleaned is true

  Scenario: Cleanup with keep_artifacts preserves artifact directory
    Given a coding coordinator with a mock worker
    And a coding job in state "succeeded" with artifacts
    When the main agent requests cleanup with keep_artifacts true
    Then the job repo directory should be removed
    But the artifact directory should be preserved

  # --- Event envelope ---

  Scenario: All events include required envelope fields
    Given a coding coordinator with a mock worker
    And a coding job that runs to completion
    When I inspect the event log
    Then every event should have v, ts, run_id, job_id, source, type, seq, and payload
    And seq numbers should be monotonically increasing per source and job_id

  Scenario: Event size is capped at 1 MiB
    Given a coding coordinator with a mock worker
    And a worker produces a tool result larger than 1 MiB
    When the event is emitted
    Then the event payload should be truncated to fit the 1 MiB limit
    And a truncation indicator should be set
