Feature: TUI live tool policy modal
  @done
  Scenario: Tool policy modal changes parent and child availability
    Given the TUI has a tool catalogue with parent and child scoped tools
    When the user opens the tool policy selector and applies changes
    Then the TUI sends live tool policy mutations
    And the updated catalogue availability is reflected in the TUI without restart

  @done
  Scenario: Tool policy shortcut is documented
    Given the TUI help is shown
    Then the help mentions Ctrl+T for tool policy
