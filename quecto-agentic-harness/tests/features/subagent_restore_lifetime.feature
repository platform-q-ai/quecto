@done @subagent-teardown @restore-lifetime
Feature: Launcher lifetime and session restore without child readoption (#1937)
  A launcher-created subagent is lifetime-scoped to the harness that launched
  it: it is started without --persist, ignores ordinary client churn because
  it is launch-bound, and cannot survive its launcher. Session restore keeps
  the transcript, workflow and past child messages but creates no operational
  child row from persisted records: no socket probe, no pid compare, no
  monitor, no readoption. The master re-spawns the workers it needs, each with
  a fresh identity and launch generation. Top-level --persist is unchanged.

  # ── The lifetime policy ────────────────────────────────────────────────────

  Scenario Outline: The harness lifetime is resolved once from persist and launch facts
    When the harness lifetime is resolved with persist "<persist>" and launched "<launched>"
    Then the resolved lifetime is "<lifetime>"

    Examples:
      | persist | launched | lifetime                      |
      | false   | false    | until_last_client_disconnects |
      | true    | false    | persistent                    |
      | false   | true     | launch_bound                  |
      | true    | true     | refused                       |

  # ── A real launcher-created child ─────────────────────────────────────────

  Scenario: A launcher-created child runs without --persist, survives client churn and dies with its launcher
    Given a live SpawnTool and AgentCmdTool backed by a mock LLM child
    When I live-spawn subagent "lifetime-worker" with initial task "say hello"
    Then the spawn result should not be an error
    And the registry entry for "lifetime-worker" holds an owned child with a launch generation
    And the child argv for "lifetime-worker" carries --parent-control but no capability material
    And the child argv for "lifetime-worker" carries no --persist
    When an ordinary probe connects to "lifetime-worker" and disconnects
    Then the child process of "lifetime-worker" is still running
    When the parent drops its control connection to "lifetime-worker"
    Then the child process of "lifetime-worker" exits within 15 seconds
    And the supervisor sent no signals to "lifetime-worker"

  # ── Session restore creates no operational child ──────────────────────────

  Scenario: Resuming a session written by an earlier harness restores history and readopts nothing
    Given a live SpawnTool and AgentCmdTool backed by a mock LLM child
    And session "restore-legacy" was saved by an earlier harness with legacy child rows naming sockets that must never be connected to
    And a restoring UDS harness with a subagent registry
    And a stale operational row is registered in the restoring harness
    When the client resumes session "restore-legacy"
    Then the resume succeeds and restores 3 messages including the past child message
    And the restored workflow run is "feature" with 2 completed steps
    And no persisted child socket was connected to
    And the operational roster of the restoring harness is empty
    And get_subagents from the restoring harness lists no children
    And no legacy child row is targetable through agent_cmd
    When the client starts a new session
    Then the operational roster of the restoring harness is empty
    And no persisted child socket was connected to
    And the saved session "restore-legacy" keeps its 3 messages and writes no child pid or socket
    When the client disconnects from the restoring harness
    Then the restoring harness exits within 10 seconds

  Scenario: After restore the master re-spawns a worker with a fresh identity and launch generation
    Given a live SpawnTool and AgentCmdTool backed by a mock LLM child
    And session "restore-respawn" was saved by an earlier harness with legacy child rows naming sockets that must never be connected to
    And a restoring UDS harness with a subagent registry
    When the client resumes session "restore-respawn"
    Then the operational roster of the restoring harness is empty
    When the restoring harness re-spawns subagent "respawned-worker" with initial task "say hello"
    Then the spawn result should not be an error
    And the registry entry for "respawned-worker" holds an owned child with a launch generation
    And the re-spawned "respawned-worker" has a fresh identity unlike every legacy row
    And no persisted child socket was connected to
    When the owned child termination of "respawned-worker" is requested
    Then the child process of "respawned-worker" exits within 15 seconds
    When the client disconnects from the restoring harness
    Then the restoring harness exits within 10 seconds

  # ── A session transition releases the departing session's children ───────
  # A launched child cannot be readopted, so a row dropped on new_session or
  # resume_session would strand a live child no teardown path can reach. The
  # departing row's monitor task owns the child's bound parent connection:
  # it is aborted before the roster is cleared, the child observes parent
  # loss and runs its own graceful shutdown (#1946). No signal is sent.
  # Interim until #1938 replaces this release with acknowledged teardown.

  Scenario: A new session releases the current session's launched child through parent loss
    Given a live SpawnTool and AgentCmdTool backed by a mock LLM child
    And session "restore-switch" was saved by an earlier harness with legacy child rows naming sockets that must never be connected to
    And a restoring UDS harness with a subagent registry
    When the client resumes session "restore-switch"
    And the restoring harness re-spawns subagent "switch-worker" with initial task "say hello"
    Then the spawn result should not be an error
    And the registry entry for "switch-worker" holds an owned child with a launch generation
    When the client starts a new session
    Then the re-spawned "switch-worker" exits gracefully via parent loss within 20 seconds
    And the operational roster of the restoring harness is empty
    And get_subagents from the restoring harness lists no children
    When the client disconnects from the restoring harness
    Then the restoring harness exits within 10 seconds

  Scenario: Resuming another session releases the current session's launched child through parent loss
    Given a live SpawnTool and AgentCmdTool backed by a mock LLM child
    And session "restore-away" was saved by an earlier harness with legacy child rows naming sockets that must never be connected to
    And session "restore-target" was saved by an earlier harness with no child rows
    And a restoring UDS harness with a subagent registry
    When the client resumes session "restore-away"
    And the restoring harness re-spawns subagent "away-worker" with initial task "say hello"
    Then the spawn result should not be an error
    And the registry entry for "away-worker" holds an owned child with a launch generation
    When the client resumes session "restore-target"
    Then the re-spawned "away-worker" exits gracefully via parent loss within 20 seconds
    And the operational roster of the restoring harness is empty
    And get_subagents from the restoring harness lists no children
    And no persisted child socket was connected to
    When the client disconnects from the restoring harness
    Then the restoring harness exits within 10 seconds
