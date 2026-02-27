@done
Feature: Coordinator Delegation via File-Based IPC
  As the main agent
  I want to delegate coding job commands to a coordinator subagent via file-based IPC
  So that the coordinator runs as a long-lived child process and the main agent stays responsive

  The coordinator is a long-lived quecto agent child process. The main agent
  communicates with it by writing JSON command files to an inbox directory and
  reading JSON response files from an outbox directory. The coordinator writes
  proactive notifications to a notifications directory and periodic state
  snapshots to state.json. A pid file tracks coordinator liveness.

  # --- Domain types ---

  Scenario: CoordinatorIpcCommand serializes to JSON
    Given a coordinator IPC command with action "run" and payload {"goal":"Fix bug","repo":"test-repo","base_ref":"main"}
    When the command is serialized to JSON
    Then the JSON should contain a "command_id" field
    And the JSON should contain an "action" field with value "run"
    And the JSON should contain a "payload" object

  Scenario: CoordinatorIpcResponse serializes to JSON
    Given a coordinator IPC response with command_id "cmd_001" and ok true
    When the response is serialized to JSON
    Then the JSON should contain "command_id" with value "cmd_001"
    And the JSON should contain "ok" with value true

  Scenario: CoordinatorIpcResponse can carry an error
    Given a coordinator IPC response with command_id "cmd_002" and error "not_found"
    When the response is serialized to JSON
    Then the JSON should contain "ok" with value false
    And the JSON should contain "error" with value "not_found"

  Scenario: CoordinatorNotification serializes with type and timestamp
    Given a coordinator notification of type "job_failed" for job "job_001"
    When the notification is serialized to JSON
    Then the JSON should contain "type" with value "job_failed"
    And the JSON should contain "job_id" with value "job_001"
    And the JSON should contain a "ts" field

  Scenario: CoordinatorState snapshot includes liveness info
    Given a coordinator state snapshot with 2 active jobs and last heartbeat "2026-01-15T10:00:00Z"
    When the state is serialized to JSON
    Then the JSON should contain "active_jobs" with value 2
    And the JSON should contain "last_heartbeat" with value "2026-01-15T10:00:00Z"
    And the JSON should contain "alive" with value true

  # --- File-based IPC infrastructure ---

  Scenario: Write command to inbox directory
    Given a coordinator IPC directory at a temp path
    When a command with action "run" is written to the inbox
    Then a JSON file should exist in the inbox directory
    And the file name should match the command_id with .json extension

  Scenario: Read command from inbox directory
    Given a coordinator IPC directory with a pending command
    When the inbox is polled for new commands
    Then the command should be returned with its action and payload
    And the command file should still exist until acknowledged

  Scenario: Write response to outbox directory
    Given a coordinator IPC directory at a temp path
    When a response for command_id "cmd_100" is written to the outbox
    Then a JSON file "cmd_100.json" should exist in the outbox directory

  Scenario: Poll outbox for response with timeout
    Given a coordinator IPC directory at a temp path
    And a response file is pre-written for command_id "cmd_200"
    When the outbox is polled for command_id "cmd_200" with timeout 1 second
    Then the response should be returned successfully

  Scenario: Poll outbox times out when no response arrives
    Given a coordinator IPC directory at a temp path
    When the outbox is polled for command_id "cmd_missing" with timeout 100ms
    Then the poll should return a timeout error

  Scenario: Remove processed inbox command
    Given a coordinator IPC directory with a pending command
    When the command is acknowledged
    Then the command file should be removed from the inbox

  Scenario: Write notification to notifications directory
    Given a coordinator IPC directory at a temp path
    When a "job_failed" notification is written for job "job_050"
    Then a JSON file should exist in the notifications directory
    And the file name should contain "job_failed"

  Scenario: Read pending notifications
    Given a coordinator IPC directory with 3 pending notifications
    When notifications are read
    Then 3 notifications should be returned
    And they should be ordered by timestamp

  Scenario: Acknowledge notification removes the file
    Given a coordinator IPC directory with a pending notification
    When the notification is acknowledged
    Then the notification file should be removed

  Scenario: Write coordinator state snapshot
    Given a coordinator IPC directory at a temp path
    When a state snapshot is written with 1 active job
    Then state.json should exist in the coordinator directory
    And reading state.json should return the snapshot with 1 active job

  Scenario: Write and read PID file
    Given a coordinator IPC directory at a temp path
    When PID 12345 is written to the pid file
    Then the pid file should contain "12345"
    And reading the pid should return 12345

  # --- Coordinator liveness ---

  Scenario: Coordinator is alive when PID file exists and process is running
    Given a coordinator IPC directory with pid file containing the current process PID
    When coordinator liveness is checked
    Then the coordinator should be reported as alive

  Scenario: Coordinator is dead when PID file contains a non-existent PID
    Given a coordinator IPC directory with pid file containing PID 999999999
    When coordinator liveness is checked
    Then the coordinator should be reported as dead

  Scenario: Coordinator is dead when pid file is missing
    Given a coordinator IPC directory at a temp path
    When coordinator liveness is checked
    Then the coordinator should be reported as dead

  # --- Notification types ---

  Scenario: Worker blocked notification contains question
    Given a coordinator notification of type "worker_blocked" for job "job_010" with detail "Which test framework?"
    Then the notification should have type "worker_blocked"
    And the notification detail should be "Which test framework?"

  Scenario: Batch complete notification lists job IDs
    Given a coordinator notification of type "batch_complete" with job_ids ["job_a","job_b","job_c"]
    Then the notification should have type "batch_complete"
    And the notification should reference 3 jobs

  Scenario: Worker stuck notification includes elapsed minutes
    Given a coordinator notification of type "worker_stuck" for job "job_020" with no_progress_minutes 30
    Then the notification should have type "worker_stuck"
    And the no_progress_minutes should be 30

  Scenario: Policy violation notification includes detail
    Given a coordinator notification of type "policy_violation" for job "job_030" with detail "worker attempted force push"
    Then the notification should have type "policy_violation"
    And the notification detail should be "worker attempted force push"
