@tui @pending
Feature: Mouse text selection shows visual highlight and clipboard feedback (#546)
  As a TUI user
  I want to see which text I'm selecting during click-and-drag
  And receive confirmation when text is copied to the clipboard

  Scenario: Selected text region has reverse-video highlight during drag
    Given the user presses mouse button at col 5 row 3
    And drags to col 20 row 3
    Then the rendered output should contain reverse-video escape codes
    And the highlight should span from col 5 to col 20 on row 3

  Scenario: Highlight spans multiple rows
    Given the user presses mouse button at col 10 row 2
    And drags to col 5 row 4
    Then rows 2 through 4 should have reverse-video highlights

  Scenario: Notification shown after clipboard copy
    Given the user selects text and releases the mouse
    When the selected text is copied to the clipboard
    Then a notification "Copied to clipboard" should appear

  Scenario: No highlight or notification on plain click
    Given the user clicks at col 5 row 3 without dragging
    Then no reverse-video highlights should be present
    And no notification should appear
