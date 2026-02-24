@pending
Feature: Crash Recovery and Event Replay
  As the coding runtime coordinator
  I want to recover state from event logs after a crash
  So that no job progress is lost and orphaned workers are handled safely

  The append-only JSONL event log is the source of truth. On startup, the
  coordinator replays event logs to reconstruct in-memory state, detects
  orphaned workers, and resumes normal operation. The jobs/index.json
  snapshot is rebuilt from logs — it is never the source of truth.

  # --- Event log replay ---

  Scenario: Coordinator recovers job state from event log on startup
    Given a job directory with an event log containing:
      | type       | state     |
      | job.start  |           |
      | job.ready  |           |
      | job.status | running   |
    When the coordinator starts up
    Then the job should be in state "running" in memory
    And the recovered state should match what the events describe

  Scenario: Coordinator recovers multiple jobs from separate event logs
    Given job directories "job_1" and "job_2" each with event logs
    And "job_1" event log ends with "job.end" state "succeeded"
    And "job_2" event log ends with "job.status" state "running"
    When the coordinator starts up
    Then "job_1" should be in state "succeeded"
    And "job_2" should be in state "running"

  Scenario: Coordinator rebuilds index from event logs
    Given a stale "jobs/index.json" that does not match current event logs
    When the coordinator starts up
    Then "jobs/index.json" should be rewritten to match the replayed state
    And the index should be consistent with the event logs

  # --- Orphaned worker detection ---

  Scenario: Coordinator detects and fails orphaned worker after crash
    Given a job event log with a "job.ready" event recording worker PID 12345
    And the event log ends with state "running" (no terminal event)
    And process 12345 is no longer alive
    When the coordinator starts up
    Then the job should transition to "failed"
    And the error_code should be "coordinator_crash"
    And a "job.end" event should be appended to the log

  Scenario: Coordinator re-attaches to still-alive worker after crash
    Given a job event log with a "job.ready" event recording a worker PID
    And the event log ends with state "running" (no terminal event)
    And the worker process is still alive
    When the coordinator starts up
    Then the coordinator should re-attach to the worker's event stream
    And the job should remain in state "running"

  # --- Ordering guarantees ---

  Scenario: Event log is flushed before index update
    Given a coding job in state "running"
    When a state transition event is processed
    Then the event should be appended and flushed to the JSONL log
    And only after the flush should the in-memory state be updated

  Scenario: Incomplete event log line is handled gracefully on replay
    Given a job event log where the last line is truncated (partial write)
    When the coordinator replays the log
    Then the truncated line should be skipped
    And recovery should proceed with the last complete event
    And a warning should be logged about the truncated line

  # --- Todo state recovery ---

  Scenario: Coordinator recovers per-worker todo state from events
    Given a job event log containing todo.create and todo.update events
    When the coordinator starts up
    Then the todo list should be reconstructed from the events
    And todo statuses should match the latest update events

  # --- Terminal state handling ---

  Scenario: Coordinator does not attempt recovery for completed jobs
    Given a job event log ending with "job.end" state "succeeded"
    When the coordinator starts up
    Then the job should be in state "succeeded"
    And no worker process check should be performed

  Scenario: Coordinator does not attempt recovery for canceled jobs
    Given a job event log ending with "job.end" state "canceled"
    When the coordinator starts up
    Then the job should be in state "canceled"
    And no recovery action should be taken
