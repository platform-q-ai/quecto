@tui @pending
Feature: Table cells with backtick-wrapped code render correctly (#550)
  As a TUI user
  I want inline code in table cells to stay within the column
  So that tables don't break their layout

  Scenario: Inline code in table cell stays in column
    Given markdown with a table containing backtick-wrapped text
    When the markdown is rendered
    Then the code text should be within its table cell
    And the table columns should be aligned

  Scenario: Mixed text and code in table cell
    Given a table cell with "Use `bash` command"
    When the markdown is rendered
    Then the full text including code should be in one cell
