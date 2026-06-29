@tui
Feature: TUI ESC abort does not break subsequent prompts
  As a TUI user
  I want to be able to abort with ESC and continue using the TUI
  So that abort is non-destructive

  Scenario: Prompt after abort receives a response
    Given the agent is streaming a response
    When the user presses Escape to abort
    And then submits a new prompt
    Then the agent should process the new prompt
    And the user should see a response

  Scenario: Pre-cancelled prompt gets feedback
    Given a stale abort fired before the prompt started
    When the TUI sends a prompt
    Then the agent should send an agent_end event
    And the TUI should not hang waiting for a response

  Scenario: Multiple aborts do not compound
    Given the user aborts 3 times in rapid succession
    When the user sends a new prompt
    Then the prompt should be processed normally
