@tui @done
Feature: TUI Ctrl+D exits the app unconditionally
  As a TUI user
  I want Ctrl+D to always exit the app
  So that I can reliably quit regardless of UI state

  Scenario: Ctrl+D exits with no overlay active
    Given the TUI is running with no overlays
    When the user presses Ctrl+D
    Then the app should set should_exit to true
    And the main loop should break

  Scenario: Ctrl+D exits even with overlay active
    Given a confirm overlay is active
    When the user presses Ctrl+D
    Then the app should exit
    And the overlay should not consume the key

  Scenario: Ctrl+D requests ordinary exit during response
    Given the agent is streaming a response
    When the user presses Ctrl+D
    Then the active agent should continue without a Ctrl-D abort
    And then the app should exit

  Scenario: Ctrl+D exits with autocomplete active
    Given the autocomplete dropdown is showing
    When the user presses Ctrl+D
    Then the app should exit
