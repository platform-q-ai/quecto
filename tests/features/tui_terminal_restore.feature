@tui
Feature: TUI restores terminal fully on exit
  As a TUI user
  I want arrow keys to work normally in my shell after quitting
  So that the TUI doesn't corrupt my terminal state

  Scenario: Terminal restored after normal exit
    Given the TUI is running in raw mode with Kitty protocol
    When the user exits with Ctrl+D
    Then the terminal should be in cooked/canonical mode
    And arrow keys should function normally in the shell
    And the cursor should be visible

  Scenario: Terminal restored after Ctrl+C exit
    Given the TUI is running
    When the process receives SIGINT
    Then the terminal should still be restored

  Scenario: Bracketed paste mode disabled on exit
    Given the TUI enabled bracketed paste mode
    When the TUI exits
    Then bracketed paste should be disabled

  Scenario: modifyOtherKeys disabled on exit
    Given the TUI enabled modifyOtherKeys mode
    When the TUI exits
    Then modifyOtherKeys should be reset to mode 0
