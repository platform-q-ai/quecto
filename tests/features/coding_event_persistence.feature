@done
Feature: JSONL Event Log Persistence
  As the coding runtime coordinator
  I want to persist all events to append-only JSONL logs
  So that crash recovery replays logs to reconstruct state and no events are lost

  Each job has its own append-only JSONL event log file at
  jobs/<job-id>/events.jsonl. The event log is the source of truth.
  The jobs/index.json snapshot is rebuilt from logs on startup and only
  written periodically for fast status queries.

  # --- Event append ---

  Scenario: Event is appended to JSONL log on state transition
    Given a coding event store for job "job_000001"
    When the coordinator appends a state transition event
    Then the event log should contain 1 valid JSON line
    And the line should be a valid EventEnvelope JSON

  Scenario: Event log is flushed to disk before in-memory state update
    Given a coding event store for job "job_000001"
    When the coordinator appends a "job.end" event
    Then the event log file should exist on disk
    And the flushed event should be readable from disk

  Scenario: Multiple events for the same job append sequentially
    Given a coding event store for job "job_000001" with events job.start, job.ready, job.status
    Then the event log should contain 3 valid JSON lines
    And each line should have monotonically increasing seq numbers
    And the file should not contain any blank lines

  # --- Index snapshot ---

  Scenario: Index snapshot is written periodically
    Given a coding event store with 5 jobs in various states
    When the coordinator writes a periodic index snapshot
    Then the index file should contain 5 entries with job_id and state
    And the snapshot should match the current in-memory state

  Scenario: Index snapshot is rebuilt from event logs on startup
    Given a coding event store for job "job_000001" with events ending in succeeded
    When the coordinator replays logs and rebuilds the index
    Then the index should show job_000001 as "succeeded"

  Scenario: Missing index is created from event logs on startup
    Given a coding event store for job "job_000001" with events but no index file
    When the coordinator replays logs and rebuilds the index
    Then the index file should be created
    And the rebuilt index should contain all discovered jobs

  # --- Log replay ---

  Scenario: Coordinator replays event log to reconstruct running job
    Given a coding event store for job "job_000001" with events job.start, job.ready, job.status
    When the coordinator replays the event log
    Then the replayed state should be "running"
    And the replayed worker_pid should be 1234
    And the replayed progress should be 50

  Scenario: Coordinator replays event log to reconstruct succeeded job
    Given a coding event store for job "job_000001" with events ending in succeeded
    When the coordinator replays the event log
    Then the replayed state should be "succeeded"
    And the replayed summary should be "all tests pass"

  Scenario: Coordinator replays todo events
    Given a coding event store for job "job_000001" with todo.create and todo.update events
    When the coordinator replays the event log
    Then the replayed log should contain todo events for reconstruction

  Scenario: Coordinator replays spawn events
    Given a coding event store for job "job_000001" with spawn.request and spawn.decision events
    When the coordinator replays the event log
    Then the replayed log should contain spawn events for reconstruction

  # --- Truncated and corrupted logs ---

  Scenario: Truncated last line is skipped during replay
    Given a coding event store for job "job_000001" with a truncated last line
    When the coordinator replays the event log
    Then the truncated line should be detected as corrupt
    And the replayed state should reflect the last complete event

  Scenario: Corrupted line in the middle of log is skipped
    Given a coding event store for job "job_000001" with a corrupted line 3
    When the coordinator replays the event log
    Then the corrupted line should be detected and skipped
    And the valid lines should still be replayed

  # --- Event log size ---

  Scenario: Event log line exceeding 1 MiB is rejected
    Given a coding event store for job "job_000001"
    When the coordinator attempts to append an event exceeding 1 MiB
    Then the oversized event should not appear in the log
    And the event log should remain valid

  # --- Job directory discovery ---

  Scenario: Coordinator discovers job directories by scanning for events.jsonl
    Given a coding event store with job directories job_000001, job_000002, job_000003
    And job_000001 and job_000003 have events.jsonl files
    But job_000002 has no events.jsonl
    When the coordinator scans for job directories
    Then job_000001 and job_000003 should be discovered
    And job_000002 should not be discovered

  # --- Concurrent coordinator protection ---

  Scenario: Coordinator acquires lock on startup
    Given a coding event store with no lock file
    When the coordinator acquires the lock
    Then the lock file should exist with the current PID

  Scenario: Second coordinator instance fails to acquire lock
    Given a coding event store with a lock held by a live process
    When a second coordinator attempts to acquire the lock
    Then the lock acquisition should fail

  # --- Event envelope validation ---

  Scenario: All persisted events include required envelope fields
    Given a coding event store for job "job_000001" with several events
    When the event log is read back
    Then every line should include v, ts, run_id, job_id, source, event_type, seq, and payload
    And the v field should be "1.0"
    And seq numbers should be monotonically increasing within each scope
