@pending
Feature: Coding Job Lifecycle
  As the coding runtime coordinator
  I want to manage coding jobs through a well-defined state machine
  So that jobs are started, tracked, and cleaned up reliably

  The coordinator exposes a command API (run/status/cancel/cleanup/list) and
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
    And no events should be emitted yet

  Scenario: Job start event is emitted when preparation begins
    Given a coding coordinator with a mock worker
    And a coding job in state "queued"
    When the coordinator begins preparation
    Then a "job.start" event should be emitted with the goal, base_ref, and branch
    And the job state should transition to "preparing"

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

  Scenario: Run command accepts priority and labels
    Given a coding coordinator with a mock worker
    When the main agent requests a coding job with priority "high" and labels ["urgent", "bugfix"]
    Then the coordinator should accept the job
    And the job metadata should reflect priority "high" and labels ["urgent", "bugfix"]

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
    Then a "job.resumed" event should be emitted with the reason
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

  Scenario: Preparing job transitions to failed on unrecoverable clone error
    Given a coding coordinator with a mock worker
    And a coding job in state "preparing"
    When the mirror clone fails with disk full error
    Then a "job.end" event should be emitted with state "failed"
    And the error_code should be "internal"

  Scenario: Cancel a job while it is preparing
    Given a coding coordinator with a mock worker
    And a coding job in state "preparing"
    When the main agent cancels the job
    Then a "job.cancel" event should be emitted with reason "user_request"
    And the job state should be "canceled"

  Scenario: Blocked job transitions to failed on unresolvable condition
    Given a coding coordinator with a mock worker
    And a coding job in state "blocked"
    When the blocking condition is determined to be permanent
    Then a "job.end" event should be emitted with state "failed"
    And the error_code should indicate the unresolvable condition

  Scenario: Cancel a blocked job
    Given a coding coordinator with a mock worker
    And a coding job in state "blocked"
    When the main agent cancels the job
    Then a "job.cancel" event should be emitted with reason "user_request"
    And the job state should be "canceled"

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
    Then a "job.cancel" event should be emitted with reason "user_request"
    And the job state should be "canceled"
    And no worker process should have been launched

  Scenario: Job is canceled on wall timeout
    Given a coding coordinator with a mock worker
    And a coding job with max_wall_seconds 5
    When the job exceeds the wall timeout
    Then a "job.cancel" event should be emitted with reason "wall_timeout"
    And the job state should be "canceled"

  Scenario: Job is canceled due to resource limit violation
    Given a coding coordinator with a mock worker
    And a coding job in state "running"
    When the worker exceeds the cgroup memory limit
    Then a "job.cancel" event should be emitted with reason "resource_limit"
    And the job state should be "canceled"

  Scenario: Cancel an already-canceled job is idempotent
    Given a coding coordinator with a mock worker
    And a coding job in state "canceled"
    When the main agent cancels the job
    Then the job state should remain "canceled"
    And no additional events should be emitted

  Scenario: Cancel a succeeded job returns current state
    Given a coding coordinator with a mock worker
    And a coding job in state "succeeded"
    When the main agent cancels the job
    Then the cancel response should return state "succeeded"
    And no "job.cancel" event should be emitted

  # --- Error codes ---

  Scenario: Job fails due to OOM kill
    Given a coding coordinator with a mock worker
    And a coding job in state "running"
    When the worker is killed by cgroup memory limit
    Then a "job.end" event should be emitted with state "failed"
    And the error_code should be "oom"

  Scenario: Job fails due to seccomp violation
    Given a coding coordinator with a mock worker
    And a coding job in state "running"
    When the worker attempts a blocked syscall
    Then a "job.end" event should be emitted with state "failed"
    And the error_code should be "seccomp_violation"

  Scenario: Job fails due to LLM refusal
    Given a coding coordinator with a mock worker
    And a coding job in state "running"
    When the LLM provider refuses to generate code
    Then a "job.end" event should be emitted with state "failed"
    And the error_code should be "llm_refusal"

  Scenario: Job fails due to internal coordinator error
    Given a coding coordinator with a mock worker
    And a coding job in state "running"
    When the coordinator encounters an unexpected internal error
    Then a "job.end" event should be emitted with state "failed"
    And the error_code should be "internal"

  Scenario: Job fails due to tool-level timeout
    Given a coding coordinator with a mock worker
    And a coding job in state "running"
    When the worker's tool execution exceeds its own timeout repeatedly
    Then a "job.end" event should be emitted with state "failed"
    And the error_code should be "timeout"
    And the event should include duration_ms

  Scenario: Job is canceled due to coordinator policy violation
    Given a coding coordinator with a mock worker
    And a coding job in state "running"
    When the coordinator detects a policy violation during execution
    Then a "job.cancel" event should be emitted with reason "coordinator_policy"
    And the job state should be "canceled"

  # --- Optional payload field coverage ---

  Scenario: Job ready event includes clone duration
    Given a coding coordinator with a mock worker
    And a coding job in state "preparing"
    When the clone completes in 1200 milliseconds and the worker starts
    Then a "job.ready" event should be emitted with clone_duration_ms 1200

  Scenario: Job blocked event includes needs field for main-agent action
    Given a coding coordinator with a mock worker
    And a coding job in state "running"
    When the worker encounters an ambiguous requirement
    Then a "job.blocked" event should be emitted with reason and needs "main-agent decision"

  Scenario: Job cancel event includes initiated_by field
    Given a coding coordinator with a mock worker
    And a coding job in state "running"
    When the main agent cancels the job
    Then a "job.cancel" event should be emitted with reason "user_request" and initiated_by "user"

  Scenario: Job end event includes duration_ms
    Given a coding coordinator with a mock worker
    And a coding job in state "running"
    When the worker completes successfully after 45000 milliseconds
    Then a "job.end" event should be emitted with state "succeeded"
    And the event should include duration_ms

  # --- Run command optional field coverage ---

  Scenario: Run command accepts profile parameter
    Given a coding coordinator with a mock worker
    When the main agent requests a coding job with profile "backend"
    Then the coordinator should accept the job
    And the job metadata should reflect profile "backend"

  Scenario: Run command accepts low priority
    Given a coding coordinator with a mock worker
    When the main agent requests a coding job with priority "low"
    Then the coordinator should accept the job
    And the job metadata should reflect priority "low"

  Scenario: Run command uses medium priority by default
    Given a coding coordinator with a mock worker
    When the main agent requests a coding job without specifying priority
    Then the coordinator should accept the job
    And the job metadata should reflect priority "medium"

  Scenario: Run command with skills parameter on successful job
    Given a coding coordinator with a mock worker
    And skill policy allows ["rust-style", "test-first"]
    When the main agent requests a coding job with skills ["rust-style"]
    Then the coordinator should accept the job
    And the skills should be applied to the worker context

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

  Scenario: Query status includes artifacts list for completed job
    Given a coding coordinator with a mock worker
    And a coding job in state "succeeded" with artifacts ["patch_001", "test_output_001"]
    When the main agent queries job status
    Then the response should include artifacts ["patch_001", "test_output_001"]

  Scenario: Query status of a non-existent job returns error
    Given a coding coordinator with a mock worker
    When the main agent queries status for job_id "nonexistent"
    Then the status command should return an error indicating job not found

  # --- Cleanup command ---

  Scenario: Cleanup a succeeded job removes directory
    Given a coding coordinator with a mock worker
    And a coding job in state "succeeded"
    When the main agent requests cleanup with keep_artifacts false
    Then the job directory should be removed
    And the response should include job_id and cleaned is true

  Scenario: Cleanup with keep_artifacts preserves artifact directory
    Given a coding coordinator with a mock worker
    And a coding job in state "succeeded" with artifacts
    When the main agent requests cleanup with keep_artifacts true
    Then the job repo directory should be removed
    But the artifact directory should be preserved

  Scenario: Cleanup a failed job removes directory
    Given a coding coordinator with a mock worker
    And a coding job in state "failed"
    When the main agent requests cleanup
    Then the job directory should be removed
    And the response should indicate cleaned is true

  Scenario: Cleanup a canceled job removes directory
    Given a coding coordinator with a mock worker
    And a coding job in state "canceled"
    When the main agent requests cleanup
    Then the job directory should be removed

  Scenario: Cleanup a running job is rejected with job_not_terminal error
    Given a coding coordinator with a mock worker
    And a coding job in state "running"
    When the main agent requests cleanup
    Then the cleanup command should fail with error code "job_not_terminal"
    And the job directory should still exist

  Scenario: Cleanup a non-existent job returns error
    Given a coding coordinator with a mock worker
    When the main agent requests cleanup for job_id "nonexistent"
    Then the cleanup command should return an error indicating job not found

  Scenario: Status of a cleaned-up job returns terminal state
    Given a coding coordinator with a mock worker
    And a coding job in state "succeeded" that has been cleaned up
    When the main agent queries job status
    Then the response should include state "succeeded" from the event log

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

  Scenario: Event envelope v field matches version pattern
    Given a coding coordinator with a mock worker
    And a coding job that emits events
    When I inspect the event log
    Then every event v field should match the pattern "^1\.[0-9]+$"

  Scenario: Event envelope source field uses allowed values
    Given a coding coordinator with a mock worker
    And a coding job that runs to completion
    When I inspect the event log
    Then every event source should be one of "main_agent", "coordinator", "worker", "child_agent"

  Scenario: Unknown event types are ignored and logged
    Given a coding coordinator with a mock worker
    When the coordinator receives an event with type "unknown.future_event"
    Then the coordinator should log a warning
    And processing should continue normally

  Scenario: Unknown payload fields are silently ignored
    Given a coding coordinator with a mock worker
    When the coordinator receives a "job.status" event with an extra field "future_field"
    Then the coordinator should process the event normally
    And the unknown field should be ignored

  Scenario: Major version mismatch in event is rejected
    Given a coding coordinator with a mock worker
    When the coordinator receives an event with v "2.0"
    Then the coordinator should reject the event
    And an error should be logged about version mismatch

  Scenario: Worker emits periodic job.status events during execution
    Given a coding coordinator with a mock worker
    And a coding job in state "running"
    When the worker reports progress periodically
    Then "job.status" events should be emitted with state "running" and progress values
    And each status event should include a summary
