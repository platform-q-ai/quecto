@tui @done
Feature: TUI Ctrl+C clears editor first, only aborts when editor is empty (#536)
  As a TUI user
  I want Ctrl+C to clear the text input first
  So that I can discard typed text without aborting a running agent

  Scenario: Ctrl+C clears editor text when agent is running and editor has content
    Given the agent is running
    And the editor contains "some typed text"
    When the user presses Ctrl+C
    Then the editor should be empty
    And the agent should still be running

  Scenario: Ctrl+C aborts agent when agent is running and editor is empty
    Given the agent is running
    And the editor is empty
    When the user presses Ctrl+C
    Then the agent should be aborted

  Scenario: Ctrl+C clears editor text when agent is idle
    Given the agent is idle
    And the editor contains "draft message"
    When the user presses Ctrl+C
    Then the editor should be empty

  Scenario: Ctrl+C does nothing when agent is idle and editor is empty
    Given the agent is idle
    And the editor is empty
    When the user presses Ctrl+C
    Then the editor should be empty
    And the agent should still be idle
