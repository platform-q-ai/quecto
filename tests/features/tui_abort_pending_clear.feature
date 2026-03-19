@tui @pending
Feature: TUI clears pending_aborts on new AgentStart
  Issue #506: If the agent backend doesn't send AgentEnd for an aborted
  run, pending_aborts stays stale and eats the next real AgentEnd.

  Scenario: AgentStart clears pending aborts
    Given the agent was aborted (pending_aborts = 1)
    When a new AgentStart arrives
    Then pending_aborts should be 0
    And the next AgentEnd should be processed normally

  Scenario: Abort then new prompt works even without stale AgentEnd
    Given the agent is running
    When the user aborts
    And sends a new prompt
    And AgentStart arrives (no stale AgentEnd was sent)
    And the agent responds and sends AgentEnd
    Then agent_running should be false
    And the response should be visible
