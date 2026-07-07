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

  @markdown-render
  Scenario Outline: Wide table content remains readable within the viewport
    Given markdown content with a table containing a long cell value
    When the markdown is rendered in a viewport that is <width> display columns wide
    Then the markdown output should contain the complete long cell value
    And every markdown output line should fit within the viewport

    Examples:
      | width |
      | 31    |
      | 32    |
      | 33    |

  @markdown-render
  Scenario Outline: Long first-column cell wraps within its column without displacing later columns
    Given markdown content with a three column table whose first cell is a long value
    When the markdown is rendered in a viewport that is <width> display columns wide
    Then every markdown output line should fit within the viewport
    And the later table columns should stay aligned under their headers

    Examples:
      | width |
      | 31    |
      | 33    |
      | 40    |

  @markdown-render
  Scenario: Markdown blocks keep visible text while stripping unsafe links and code language controls
    Given markdown content with a heading, quote, list, unsafe link, and code fence
    When the markdown is rendered at width 80
    Then the markdown output should contain "Release notes"
    And the markdown output should contain "quoted warning"
    And the markdown output should contain "first item"
    And the markdown output should contain "safe link"
    And the markdown output should not contain "evil-title"
    And no source OSC control sequences should appear in markdown output
