@tui
Feature: TUI context size and usage percentage display
  As a TUI user
  I want to see context usage percentage in the footer
  So that I know how close I am to the context window limit

  Scenario: Token usage displayed after agent response
    Given the agent completes a response using 5000 input tokens
    And the context window is 200k tokens
    When the TurnEnd event includes usage data
    Then the footer should show "2.5%/200k"

  Scenario: Usage updates after each turn
    Given the agent has processed multiple turns
    When each TurnEnd event includes cumulative usage
    Then the footer should reflect the latest usage percentage

  Scenario: High usage shows warning color
    Given context usage exceeds 70%
    When the footer renders
    Then the usage should be displayed in warning color

  Scenario: Critical usage shows error color
    Given context usage exceeds 90%
    When the footer renders
    Then the usage should be displayed in error color
