@done
Feature: Coordinator entrypoint
  The `quecto coordinator` command runs a long-lived inbox polling loop
  that processes coding job commands via file-based IPC. It builds the
  full CodingJobService stack, reads commands from inbox, dispatches them,
  writes responses to outbox, and exits on shutdown command.

  # ── Argument parsing ────────────────────────────────────────────────

  Scenario: Parse coordinator args with IPC directory
    Given coordinator args "--ipc-dir /tmp/coord"
    When the coordinator args are parsed
    Then the coordinator ipc_dir should be "/tmp/coord"
    And the coordinator poll_interval_ms should be 500

  Scenario: Parse coordinator args with custom poll interval
    Given coordinator args "--ipc-dir /tmp/coord --poll-interval-ms 100"
    When the coordinator args are parsed
    Then the coordinator poll_interval_ms should be 100

  Scenario: Parse coordinator args missing required ipc-dir
    Given coordinator args "--poll-interval-ms 200"
    When the coordinator args are parsed
    Then the coordinator parse should fail with "missing required flag --ipc-dir"

  Scenario: Parse coordinator args with unknown flag
    Given coordinator args "--ipc-dir /tmp/coord --unknown-flag"
    When the coordinator args are parsed
    Then the coordinator parse should fail with "unknown flag"

  # ── Tick loop integration ──────────────────────────────────────────

  Scenario: Coordinator tick processes a list command
    Given a coordinator entrypoint with mock service
    And a pending coordinator inbox command "list" with payload:
      """
      {}
      """
    When the coordinator runs one tick
    Then the coordinator tick should process 1 command
    And the coordinator tick should not request shutdown

  Scenario: Coordinator tick processes a shutdown command
    Given a coordinator entrypoint with mock service
    And a pending coordinator inbox command "shutdown" with payload:
      """
      {}
      """
    When the coordinator runs one tick
    Then the coordinator tick should process 1 command
    And the coordinator tick should request shutdown

  Scenario: Coordinator tick processes multiple commands
    Given a coordinator entrypoint with mock service
    And a pending coordinator inbox command "list" with payload:
      """
      {}
      """
    And a pending coordinator inbox command "list" with payload:
      """
      {}
      """
    When the coordinator runs one tick
    Then the coordinator tick should process 2 commands

  Scenario: Coordinator tick with empty inbox
    Given a coordinator entrypoint with mock service
    When the coordinator runs one tick
    Then the coordinator tick should process 0 commands
    And the coordinator tick should not request shutdown

  Scenario: Coordinator writes PID on startup
    Given a coordinator entrypoint with mock service
    When the coordinator writes its PID
    Then the coordinator PID file should contain the current process PID

  Scenario: Coordinator writes alive state after tick
    Given a coordinator entrypoint with mock service
    And a pending coordinator inbox command "list" with payload:
      """
      {}
      """
    When the coordinator runs one tick
    Then the coordinator state should show alive
