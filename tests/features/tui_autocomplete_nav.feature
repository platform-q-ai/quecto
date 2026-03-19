@tui
Feature: TUI slash command menu arrow navigation
  As a TUI user
  I want arrow keys to navigate the slash command menu sequentially
  So that I can select commands reliably

  Scenario: Down arrow advances through suggestions
    Given the editor text is "/"
    And the autocomplete shows all commands
    When the user presses Down 3 times
    Then the selected index should be 3

  Scenario: Up arrow navigates backwards
    Given the editor text is "/"
    And the autocomplete shows all commands
    And the selected index is 3
    When the user presses Up
    Then the selected index should be 2

  Scenario: Update with same text does not reset selection
    Given the editor text is "/"
    And the selected index is 2
    When update is called again with "/"
    Then the selected index should remain 2
