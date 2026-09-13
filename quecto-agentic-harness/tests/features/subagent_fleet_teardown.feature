@done @subagent-teardown @fleet-teardown
Feature: Fleet teardown and harness exit transitions converge on one use case (#1938)
  Every path that ends a harness's whole fleet of direct children — an
  operator's delete_all_subagents (idle or busy), SIGTERM/SIGINT, the last
  client of the default lifetime disconnecting, and a session transition —
  runs the one TerminateAllDelegatedAgents use case: each direct child is
  claimed, asked to shut down over its one edge, concluded through the
  owned-handle fallback and compensated exactly once, under a concurrency
  bound; exited tombstones are pruned so no live operational roster is ever
  persisted. Concurrent triggers join one run. A child that cannot be
  settled is reported, never silently dropped. No interface code drains the
  registry and no termination path signals a pid.

  # ── The use case over port fakes ──────────────────────────────────────────

  Scenario: Every direct child is settled once and the tombstones are pruned
    Given a harness owning direct children A and D with A's own children B and C
    When the fleet teardown runs
    Then children "A, D" settled "graceful" and none is unsettled
    And every direct child was claimed, asked once, concluded and compensated in that order
    And the tombstones were pruned and the roster is empty

  Scenario: Two triggers arriving together join one fleet run
    Given a harness owning direct children A and D with A's own children B and C
    And the shutdown of child "A" is held open
    When the fleet teardown is triggered twice at once
    Then both triggers observe one run and no child was asked twice

  Scenario: A child that survives its fallback is reported unsettled with its claim lifted
    Given a harness owning direct children A and D with A's own children B and C
    And child "A" survives even the owned-handle fallback
    When the fleet teardown runs
    Then child "A" is reported unsettled with its claim lifted and "D" settled

  Scenario: A child another termination owns is joined rather than asked again
    Given a harness owning direct children A and D with A's own children B and C
    And an operator kill already claimed child "A"
    When the operator kill of child "A" finishes while the fleet teardown runs
    Then child "A" was joined, never asked, and "D" settled gracefully

  Scenario: A dropped fleet run is reported interrupted and the next trigger starts afresh
    Given a harness owning direct children A and D with A's own children B and C
    Then the fleet run is reported interrupted and the next run starts afresh

  # ── The harness shutdown drives the fleet ─────────────────────────────────

  Scenario Outline: The common shutdown settles the fleet before persistence and exit readiness
    Given a harness owning direct children A and D with A's own children B and C
    When the harness shutdown is admitted and executed for "<reason>"
    Then the shutdown cancelled the turn, settled the fleet, persisted and signalled exit in that order

    Examples:
      | reason              |
      | termination_signal  |
      | operator_request    |
      | parent_shutdown     |

  Scenario: A survivor does not stop the harness from exiting
    Given a harness owning direct children A and D with A's own children B and C
    And child "D" survives even the owned-handle fallback
    When the harness shutdown is admitted and executed for "termination_signal"
    Then the shutdown reports the survivor as failed and still completes

  # ── Spawn admission against the freeze ────────────────────────────────────

  Scenario: A spawn registering after the shutdown froze the harness is refused
    When a spawn is registered against a frozen harness
    Then the spawn is refused because the harness is frozen

  Scenario: A spawn registering before any shutdown is admitted
    When a spawn is registered against an accepting harness
    Then the spawn is admitted

  # ── Busy delete-all over the production adapters ──────────────────────────

  @serial
  Scenario: A busy parent answers delete_all_subagents with the settled fleet
    When a busy client sends delete_all_subagents for 2 launched rows with unreachable sockets
    Then the busy delete response reports 2 removed as unobserved and the registry is empty

  # ── Real entry points: an in-process harness with a real child ────────────

  @serial
  Scenario: An idle delete_all_subagents tears down a live child gracefully
    Given a live SpawnTool and AgentCmdTool backed by a mock LLM child
    And session "fleet-delete" was saved by an earlier harness with no child rows
    And a restoring UDS harness with a subagent registry
    When the client resumes session "fleet-delete"
    And the restoring harness re-spawns subagent "delete-worker" with initial task "say hello"
    Then the spawn result should not be an error
    When the client sends delete_all_subagents to the restoring harness
    Then the delete response reports 1 removed with every child settled
    And the re-spawned "delete-worker" exits gracefully via the fleet teardown within 20 seconds
    And the operational roster of the restoring harness is empty
    And get_subagents from the restoring harness lists no children
    When the client disconnects from the restoring harness
    Then the restoring harness exits within 10 seconds

  @serial
  Scenario: A termination signal tears the fleet down before the harness exits
    Given a live SpawnTool and AgentCmdTool backed by a mock LLM child
    And session "fleet-signal" was saved by an earlier harness with no child rows
    And a restoring UDS harness with a subagent registry
    When the client resumes session "fleet-signal"
    And the restoring harness re-spawns subagent "signal-worker" with initial task "say hello"
    Then the spawn result should not be an error
    When a termination signal is delivered to the restoring harness
    Then the re-spawned "signal-worker" exits gracefully via the fleet teardown within 20 seconds
    And the restoring harness exits within 20 seconds
    And no saved session writes a live child

  @serial
  Scenario: The last client of the default lifetime tears the fleet down before the process returns
    Given a live SpawnTool and AgentCmdTool backed by a mock LLM child
    And session "fleet-last-client" was saved by an earlier harness with no child rows
    And a restoring UDS harness with a subagent registry
    When the client resumes session "fleet-last-client"
    And the restoring harness re-spawns subagent "exit-worker" with initial task "say hello"
    Then the spawn result should not be an error
    When the client disconnects from the restoring harness
    Then the re-spawned "exit-worker" exits gracefully via the fleet teardown within 20 seconds
    And the restoring harness exits within 20 seconds
    And no saved session writes a live child

  @serial
  Scenario: A persistent harness survives its last client and keeps its child
    Given a live SpawnTool and AgentCmdTool backed by a mock LLM child
    And session "fleet-persist" was saved by an earlier harness with no child rows
    And a persistent restoring UDS harness with a subagent registry
    When the client resumes session "fleet-persist"
    And the restoring harness re-spawns subagent "persist-worker" with initial task "say hello"
    Then the spawn result should not be an error
    When the client disconnects from the restoring harness
    Then the restoring harness is still serving after 2 seconds
    And the child process of "persist-worker" is still running
    When a termination signal is delivered to the restoring harness
    Then the re-spawned "persist-worker" exits gracefully via the fleet teardown within 20 seconds
    And the restoring harness exits within 20 seconds

  @serial
  Scenario: A new session finalizes a script-managed member through its retained kill argv
    Given a live SpawnTool and AgentCmdTool backed by a mock LLM child
    And session "fleet-member" was saved by an earlier harness with no child rows
    And a restoring UDS harness with a subagent registry
    When the client resumes session "fleet-member"
    Given a script-managed member with a fake kill argv is registered in the restoring harness
    When the client starts a new session
    Then the fake kill argv of the script-managed member ran once
    And the operational roster of the restoring harness is empty
    When the client disconnects from the restoring harness
    Then the restoring harness exits within 10 seconds
