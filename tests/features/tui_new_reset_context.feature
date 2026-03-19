@tui
Feature: /new and /clear reset context window display
  As a TUI user
  I want /new to reset the context usage in the footer
  So that I know I'm starting fresh

  Scenario: /new resets context percentage
    Given the footer shows "45.2%/200k" context usage
    When the user executes /new
    Then the footer context should reset to "?/0"

  Scenario: /clear also resets context
    Given the footer shows context usage data
    When the user executes /clear
    Then the footer context should reset
