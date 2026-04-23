@tui @pending
Feature: TUI Phase 9 — Wire Components and Fix Escape Abort
  Integration wiring and bug fixes.

  Scenario: Escape abort stops spinner and finalizes message
    Given the agent is running with a spinner active
    When the user presses Escape
    Then the spinner should stop
    And the streaming [message] should be finalized
    And a status [message] "Operation aborted" should appear
    And the [session] should remain alive

  Scenario: Escape abort does not kill session
    Given the agent is running
    When the user presses Escape
    And the user sends a new prompt "hello"
    Then the agent should process the new prompt

  Scenario: Autocomplete activates on slash
    Given the editor text is "/"
    Then the autocomplete dropdown should be visible

  Scenario: Autocomplete Tab accepts suggestion
    Given the autocomplete is showing "/model"
    When the user presses Tab
    Then the editor text should be "/model"

  Scenario: Ctrl+O toggles tool output expansion
    Given collapsed tool output in the chat
    When the user presses Ctrl+O
    Then all tool outputs should be expanded

  Scenario: Notifications render above footer
    Given a success notification "Model switched"
    When the layout renders
    Then "Model switched" should appear above the footer

  Scenario: Ctrl+Z suspends and resumes
    Given the TUI is in raw mode
    When the user presses Ctrl+Z
    Then the terminal should be restored before suspend
    And raw mode should be re-entered on resume
