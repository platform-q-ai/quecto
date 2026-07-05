@tui @security @done
Feature: TUI markdown table safety and correctness
  As a TUI user
  I want markdown table cells sanitized and properly measured
  So that ANSI injection is prevented and columns align correctly

  Scenario: Table cell ANSI escapes are stripped
    Given a markdown table with cells containing "\x1b[31mred\x1b[0m"
    When the table is rendered
    Then the displayed cell text should be "red" without ANSI escapes
    And no terminal control sequences should appear in output

  Scenario: Table cell cursor repositioning is stripped
    Given a markdown table with cells containing "\x1b[H\x1b[2J"
    When the table is rendered
    Then the clear-screen sequence should not be present
    And the cell should render as empty or safe text

  Scenario: Table cell OSC hyperlink injection is stripped
    Given a markdown table with cells containing "\x1b]8;;http://evil.com\x07click\x1b]8;;\x07"
    When the table is rendered
    Then the OSC sequence should not be present
    And the cell text should be "click" or stripped equivalent

  Scenario: CJK characters in table cells use display width
    Given a markdown table with cells containing "你好世界"
    When the table column widths are calculated
    Then the column width should account for double-width CJK characters
    And the column should be at least 8 display columns wide

  Scenario: Emoji in table cells use display width
    Given a markdown table with cells containing "🎉🎊"
    When the table column widths are calculated
    Then the column width should account for double-width emoji

  Scenario: All empty table cells do not cause division by zero
    Given a markdown table where every cell is empty
    When the table is rendered
    Then no panic or division by zero should occur
    And the table should render without errors

  Scenario: Table truncation respects display width
    Given a table cell with "café" (5 bytes, 4 display chars)
    When the cell is truncated to fit column width
    Then truncation should use display width not byte length
