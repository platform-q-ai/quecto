@tui @done
Feature: TUI chat adjacency to input border
  As a TUI user
  I want the latest chat output directly above the input area
  So that the conversation uses the maximum available height

  Scenario: No blank gap when chat is short
    Given the chat has 5 lines of content
    And the terminal has 30 rows
    When the screen renders
    Then the latest chat line appears directly above the editor border

  Scenario: Chat fills screen — auto-scroll takes priority
    Given the chat fills the entire available space
    When the screen renders
    Then chat is scrolled to show the latest content
    And spacing may be reduced to fit content
