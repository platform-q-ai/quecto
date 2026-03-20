@tui @pending
Feature: TUI sci-fi workflow header bar (#563)
  As a TUI user
  I want to see workflow progress in a persistent header bar
  So that I can track the BDD/TDD workflow at a glance

  Scenario: Workflow bar renders with issue and progress
    Given a workflow state with issue 559 "kill command" and 4 of 14 done
    When the workflow bar is rendered at width 80
    Then the output should contain "559"
    And the output should contain "kill command"
    And the output should contain a progress bar

  Scenario: Workflow bar hidden when no issue set
    Given a workflow state with no issue and 0 of 14 done
    When the workflow bar is rendered at width 80
    Then the output should be empty

  Scenario: Workflow bar shows correct phase
    Given a workflow state with issue 100 "test" and 4 of 14 done
    When the workflow bar is rendered at width 80
    Then the output should contain the current phase name

  Scenario: Ctrl+Shift+A toggles auto-continue notification
    Given the TUI is connected to an agent
    When the user presses Ctrl+Shift+A
    Then a notification about auto-continue should appear

  Scenario: Ctrl+Shift+N toggles completion nudge notification
    Given the TUI is connected to an agent
    When the user presses Ctrl+Shift+N
    Then a notification about completion nudge should appear
