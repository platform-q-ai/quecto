@tui @done
Feature: TUI autocomplete Enter sets editor text before submitting
  As a TUI user
  I want Enter on an autocomplete suggestion to update the editor text
  So that the editor and submitted text are always in sync

  Scenario: Enter on autocomplete sets editor before submit
    Given the editor text is "/qu"
    And the autocomplete is showing "/quit" highlighted
    When the user presses Enter
    Then the editor text should be "/quit" before handle_submit runs
    And the submitted command should be "/quit"

  Scenario: Editor text readable during submit processing
    Given the editor text is "/mo"
    And the autocomplete is showing "/model" highlighted
    When the user presses Enter
    Then any code reading editor.text() during submit should see "/model"
    And not the stale partial "/mo"

  Scenario: Tab-accept already sets editor text correctly
    Given the editor text is "/he"
    And the autocomplete is showing "/help" highlighted
    When the user presses Tab
    Then the editor text should be "/help"
    And the autocomplete should remain active for further editing
