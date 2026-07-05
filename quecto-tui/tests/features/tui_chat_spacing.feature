@tui @done
Feature: TUI 3-line spacing between agent response and input border
  As a TUI user
  I want at least 3 blank lines between the chat and the input area
  So that the response text is visually separated from the editor

  Scenario: Minimum spacing when chat is short
    Given the chat has 5 lines of content
    And the terminal has 30 rows
    When the screen renders
    Then at least 3 blank lines should appear between chat and editor border

  Scenario: Chat fills screen — auto-scroll takes priority
    Given the chat fills the entire available space
    When the screen renders
    Then chat is scrolled to show the latest content
    And spacing may be reduced to fit content
