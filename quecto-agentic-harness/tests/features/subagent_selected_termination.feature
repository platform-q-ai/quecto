@done @subagent-teardown @selected-termination
Feature: Selected termination of a delegated agent (#1936, #1882)
  An operator's `agent_cmd kill` names one delegated agent by uuid or live
  display label. The row is claimed stopping before any effect; exactly one
  edge is routed (self shutdown to a direct child, the delegated-target
  command through the direct ancestor of a deeper one, which stays alive);
  the exit — the protocol acknowledgement and observed exit, or the owned
  handle's fallback, or for a remote agent its row's compensation — is
  observed before the terminal effects run exactly once: cleanup, membership,
  subtree removal, one survivor broadcast. Explicit kill, reaper exit,
  monitor EOF and launch rollback converge on one compensation. The result
  vocabulary is graceful, fallback, already-exited or failed.

  # ── Survivor projection ──────────────────────────────────────────────────

  Scenario: Killing a nested descendant is forwarded through its ancestor, which survives with its siblings
    Given a root harness whose launched child "A" acknowledges commands
    And "A" reported descendants "B" and "C" with launch generations
    And a launched child "D" that acknowledges commands
    When the operator kills "B" and "A" then reports a snapshot without "B"
    Then the kill result is "graceful" and removed "B"
    And "A" received exactly one terminate_delegated_agent command for "B" and no shutdown
    And "A", "C" and "D" remain live
    And exactly one survivor broadcast went out, listing "A", "C" and "D" only

  Scenario: Killing a directly owned child that acknowledges and exits removes its subtree without a signal
    Given a root harness whose launched child "A" acknowledges commands and exits when told
    And "A" reported descendants "B" and "C" with launch generations
    And a launched child "D" that acknowledges commands
    When the operator kills "A"
    Then the kill result is "graceful" and removed "A", "B" and "C"
    And the owned process of "A" was never signalled
    And "D" remains live
    And exactly one survivor broadcast went out, listing "D" only

  Scenario: A display label resolves to the one live agent it names
    Given a root harness whose launched child "A" acknowledges commands and exits when told
    When the operator kills "alpha"
    Then the kill result is "graceful" and removed "A"

  # ── Fallback only for a directly owned handle ────────────────────────────

  # Timing-bound: a blocking wait must not drift a co-scheduled fixture.
  @serial
  Scenario Outline: A negative acknowledgement authorises the owned handle's fallback
    Given a root harness whose launched child "A" <behaviour> and holds a sleeping process
    When the operator kills "A"
    Then the kill result is "fallback" and removed "A"
    And the owned process of "A" was signalled after the protocol outcome "<negative>"

    Examples:
      | behaviour                              | negative           |
      | refuses commands                       | refused            |
      | answers with a malformed acknowledgement | timed out        |
      | is unreachable                         | unreachable        |

  # Timing-bound: a blocking wait must not drift a co-scheduled fixture.
  @serial
  Scenario: An acknowledged owned child that never exits is ended after its exit budget
    Given a root harness whose launched child "A" acknowledges commands and holds a sleeping process
    When the operator kills "A"
    Then the kill result is "fallback" and removed "A"
    And the owned process of "A" was signalled after the protocol outcome "acknowledged but not exited"

  Scenario: A remote member this harness does not own fails truthfully when it cannot be reached
    Given a root harness whose launched child "E" is unreachable
    When the operator kills "E"
    Then the kill result is "failed"
    And "E" remains live
    And no survivor broadcast went out

  # Timing-bound: a blocking wait must not drift a co-scheduled fixture.
  @serial
  Scenario: An acknowledged remote member whose exit is never observed fails within the bound
    Given a root harness whose launched child "E" acknowledges commands
    When the operator kills "E"
    Then the kill result is "failed"
    And "E" remains live
    And no survivor broadcast went out

  # ── Stable no-effect refusals ────────────────────────────────────────────

  Scenario Outline: References that name no live delegated agent have no effect
    Given a root harness whose launched child "A" acknowledges commands
    And a launched child "D" that acknowledges commands
    And <precondition>
    When the operator kills "<reference>"
    Then the kill is refused with "<detail>"
    And "A" is not exited
    And no command reached "A"
    And no survivor broadcast went out

    Examples:
      | precondition                                                | reference | detail                   |
      | nothing else                                                | ghost     | not found                |
      | a second live row also labelled "alpha"                     | alpha     | ambiguous                |
      | "D" has already been compensated                            | D         | already exited           |
      | a fixture row "F" without a launch generation               | F         | not a delegated agent    |
      | reported rows "Y" and "Z" whose parents form a cycle         | Y         | lineage cycle            |
      | a termination of "A" already in flight                      | A         | already in flight        |

  # ── Convergence of kill, reaper, monitor and rollback ────────────────────

  # Timing-bound: a blocking wait must not drift a co-scheduled fixture.
  @serial
  Scenario: A kill racing the child's natural exit compensates and broadcasts exactly once
    Given a root harness whose launched child "A" acknowledges commands and holds a short-lived process
    And the reaper of "A" is running
    When the operator kills "A"
    Then the kill result is "graceful" and removed "A"
    And exactly one survivor broadcast went out, listing nothing
    And the row of "A" is compensated once

  # Timing-bound: a blocking wait must not drift a co-scheduled fixture.
  @serial
  Scenario: A monitor connection ending for a retained process defers to the reaper
    Given a root harness whose launched child "A" acknowledges commands and holds a sleeping process
    And the reaper of "A" is running
    When the monitor of "A" observes its connection closed
    Then the observation is deferred to the process exit and "A" remains live
    When the process of "A" ends
    Then the row of "A" is compensated once
    And exactly one survivor broadcast went out, listing nothing

  # Timing-bound: a blocking wait must not drift a co-scheduled fixture.
  @serial
  Scenario: A registered launch that fails is rolled back through the same conclusion
    Given a root harness whose launched child "A" refuses commands and holds a sleeping process
    When the registered launch of "A" is rolled back
    Then the rollback concluded "ExitedAfterFallback" and removed "A"
    And the row of "A" is compensated once

  Scenario: A duplicate rollback joins the first
    Given a root harness whose launched child "A" acknowledges commands
    When the registered launch of "A" is rolled back twice
    Then the second rollback concluded "NoRetainedHandle" and removed nothing
    And the row of "A" is compensated once

  Scenario: A shutdown during a running turn cancels it and the harness exits without a further turn
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM will delay its response by 20 seconds
    And a launch-bound UDS harness launched with a parent control credential
    When the parent connects and presents its credential
    And an ordinary client sends a prompt that the harness starts working on
    And the parent sends shutdown "selected_termination" with id "k-1"
    Then the parent receives a shutdown acknowledgement for "k-1" with reason "selected_termination"
    And the launched harness exits within 3 seconds

  # ── Interface and wire ───────────────────────────────────────────────────

  Scenario: The agent_cmd tool delegates kill to its composed owner and presents refusals
    Given a root harness whose launched child "A" acknowledges commands and exits when told
    And an AgentCmdTool over the root registry with the composed kill owner
    When I execute agent_cmd with '{"agent_id":"ghost","command":"kill"}'
    Then the agent_cmd result should be an error
    And the agent_cmd result should contain "not found"
    When I execute agent_cmd with '{"command":"kill"}'
    Then the agent_cmd result should be an error
    And the agent_cmd result should contain "missing required field: agent_id"
    When I execute agent_cmd with '{"agent_id":"a b","command":"kill"}'
    Then the agent_cmd result should be an error
    When I execute agent_cmd with '{"agent_id":"A","command":"kill"}'
    Then the agent_cmd result should not be an error
    And the agent_cmd result should contain "graceful"
    And the agent_cmd result should not contain "signalled"

  Scenario: An AgentCmdTool without a composed owner refuses kill
    Given an AgentCmdTool without a composed kill owner
    When I execute agent_cmd with '{"agent_id":"w1","command":"kill"}'
    Then the agent_cmd result should be an error
    And the agent_cmd result should contain "kill is not available"

  Scenario: Reported snapshots carry the launch generation the root routes by
    Given a root harness whose launched child "A" acknowledges commands
    When "A" forwards a snapshot listing "B" with launch generation 4
    Then the root's lineage lists "B" at generation 4 beneath "A"
    And the root's own snapshot carries launch generations for "A" and "B"
    And a kill of "B" is forwarded to "A" at generation 4

  Scenario: The kill vocabulary renders every outcome and error
    Then every termination result and kill error renders its vocabulary
