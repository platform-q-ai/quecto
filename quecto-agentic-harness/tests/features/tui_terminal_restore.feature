@tui @done
Feature: TUI restores terminal fully on exit
  As a TUI user
  I want arrow keys to work normally in my shell after quitting
  So that the TUI doesn't corrupt my terminal state

  Scenario: modifyOtherKeys disabled on exit
    Given the TUI enabled modifyOtherKeys mode
    When the TUI exits
    Then modifyOtherKeys should be reset to mode 0
