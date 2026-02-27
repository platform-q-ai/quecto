@done
Feature: Coordinator Spawn and Liveness
  As the main agent
  I want to auto-spawn the coordinator subagent if it is not alive
  So that coding_job commands are always handled by a running coordinator

  The main agent checks coordinator liveness before dispatching each command.
  If the coordinator is dead or has never been started, the spawner launches
  a new `quecto coordinator` child process with an IPC directory,
  writes its PID to coordinator/pid, and returns. All communication then
  proceeds via file-based IPC.

  # --- Domain port: CoordinatorSpawner trait ---

  Scenario: Spawner trait reports already alive when coordinator is running
    Given a mock coordinator spawner that reports alive
    When the spawner is asked to ensure the coordinator is alive
    Then the spawner should return the existing PID

  Scenario: Spawner trait spawns a new coordinator when none is running
    Given a mock coordinator spawner that reports not alive
    When the spawner is asked to ensure the coordinator is alive
    Then the spawner should return a new PID
    And the spawner should have launched exactly 1 process

  Scenario: Spawner trait returns error when spawn fails
    Given a mock coordinator spawner that fails to spawn
    When the spawner is asked to ensure the coordinator is alive
    Then the spawner should return an error containing "spawn failed"

  # --- Integration with delegation tool ---

  Scenario: Delegation tool auto-spawns coordinator before dispatching command
    Given a coordinator delegation tool with auto-spawn enabled
    And the mock spawner reports the coordinator is not alive
    And the mock IPC will respond with ok true and body {"state":"queued","job_id":"j1"}
    When I execute the delegation tool with action "run" and payload {"goal":"test","repo":"r","base_ref":"main"}
    Then the spawner should have been called to ensure alive
    And the delegation tool result should not be an error
    And the delegation tool result should contain "job_id"

  Scenario: Delegation tool skips spawn when coordinator is already alive
    Given a coordinator delegation tool with auto-spawn enabled
    And the mock spawner reports the coordinator is alive with PID 12345
    And the mock IPC will respond with ok true and body {"jobs":[]}
    When I execute the delegation tool with action "list" and payload {}
    Then the spawner should have been called to ensure alive
    And the spawner should not have launched any process
    And the delegation tool result should not be an error

  Scenario: Delegation tool returns error when auto-spawn fails
    Given a coordinator delegation tool with auto-spawn enabled
    And the mock spawner fails to spawn
    When I execute the delegation tool with action "status" and payload {"job_id":"j1"}
    Then the delegation tool result should be an error
    And the delegation tool result should contain "spawn"

  # --- Spawn configuration ---

  Scenario: Spawner uses configurable poll interval
    Given a coordinator process spawner with poll interval 100 ms
    Then the spawner poll interval should be 100

  Scenario: Spawner uses default poll interval of 500 ms
    Given a coordinator process spawner with default config
    Then the spawner poll interval should be 500
