@tui @pending
Feature: TUI ESC abort does not break subsequent prompts
  Issue #502: After pressing ESC to abort, the agent fails to respond
  to any subsequent prompts. Root cause is a race between the stale
  AgentEnd from the aborted run and the new AgentStart.

  Scenario: Abort followed by new prompt receives response
    Given the agent is running (agent_running is true)
    When the user presses Escape to abort
    And the user submits a new prompt
    And AgentStart event arrives for the new prompt
    Then agent_running should be true

  Scenario: Stale AgentEnd from aborted run is ignored
    Given the agent was aborted (generation 1)
    And a new prompt started (generation 2)
    When an AgentEnd event arrives for generation 1
    Then agent_running should still be true
    And the spinner should still be active

  Scenario: AgentEnd for current generation is processed
    Given the agent is running at generation 2
    When an AgentEnd event arrives for generation 2
    Then agent_running should be false
    And the spinner should be stopped

  Scenario: Handle abort does not prematurely clear agent_running
    Given the agent is running
    When handle_abort is called
    Then an abort command should be sent
    And the spinner should stop
    And the chat should show "Operation aborted"
    But agent_running should remain true until AgentEnd arrives

  Scenario: Multiple rapid aborts do not corrupt state
    Given the agent is running at generation 1
    When the user aborts twice rapidly
    And a new prompt is sent (generation 3)
    And AgentEnd arrives for generation 1
    And AgentEnd arrives for generation 2
    Then agent_running should still be true for generation 3

  Scenario: Normal non-aborted flow still works
    Given the agent is idle
    When the user submits a prompt
    And AgentStart arrives
    And tokens arrive
    And AgentEnd arrives for the current generation
    Then agent_running should be false
    And the response should be displayed
