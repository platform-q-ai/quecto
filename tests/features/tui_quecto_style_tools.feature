@tui @pending
Feature: TUI Quecto-style tool output — unified entries, bg colors, tool-specific formatting
  Issue #510: Tool output should use background colors, tool-specific
  formatting, and collapsed previews matching Quecto's rendering.

  # ---------------------------------------------------------------------------
  # Unified tool entry (no more ToolStart/ToolEnd duplication)
  # ---------------------------------------------------------------------------

  Scenario: Running tool renders with pending background and header
    Given a chat with a running bash tool for "ls -la"
    When rendered
    Then the output should show "$ ls -la" as the header
    And every content line should have a pending background color

  Scenario: Completed tool updates in place (no duplicate entry)
    Given a chat with a running bash tool for "ls -la"
    When the tool completes with output "file.txt"
    Then there should be exactly one tool block in the chat
    And the header should still show "$ ls -la"
    And the output should show "file.txt"

  # ---------------------------------------------------------------------------
  # Tool-specific formatting
  # ---------------------------------------------------------------------------

  Scenario: Bash tool shows command as header with output tail
    Given a completed bash tool with command "cargo test" and 50 lines of output
    When rendered collapsed
    Then the header should show "$ cargo test"
    And the last 5 lines of output should be visible
    And a count "... (45 earlier lines, Ctrl+O to expand)" should appear

  Scenario: Read tool shows file path and content preview
    Given a completed read tool for "src/main.rs" with 30 lines
    When rendered collapsed
    Then the header should show "read src/main.rs"
    And the first 10 lines should be visible
    And a count "... (20 more lines, Ctrl+O to expand)" should appear

  Scenario: Write tool shows file path and content
    Given a completed write tool for "src/lib.rs" with content
    When rendered collapsed
    Then the header should show "write src/lib.rs"
    And a content preview should be visible

  Scenario: Edit tool shows file path and diff
    Given a completed edit tool for "src/main.rs" with diff
    When rendered
    Then the header should show "edit src/main.rs"
    And added lines should be green
    And removed lines should be red

  Scenario: Generic tool shows name and args
    Given a completed "web_fetch" tool
    When rendered
    Then the header should show the tool name in bold

  # ---------------------------------------------------------------------------
  # Background colors
  # ---------------------------------------------------------------------------

  Scenario: Running tool has pending background
    Given a running tool
    Then content lines should have bg color 236 (dark gray)

  Scenario: Successful tool has success background
    Given a completed successful tool
    Then content lines should have bg color 22 (dark green)

  Scenario: Failed tool has error background
    Given a completed failed tool
    Then content lines should have bg color 52 (dark red)

  # ---------------------------------------------------------------------------
  # Expand / collapse
  # ---------------------------------------------------------------------------

  Scenario: Ctrl+O expands tool to show full output
    Given a collapsed bash tool with 50 lines of output
    When the tool is expanded
    Then all 50 lines should be visible

  Scenario: Expanded tool shows collapse hint
    Given an expanded bash tool with 50 lines
    Then no "earlier lines" count should appear

  # ---------------------------------------------------------------------------
  # Width compliance
  # ---------------------------------------------------------------------------

  Scenario: Tool output respects terminal width
    Given a completed bash tool with long output
    When rendered at width 40
    Then no line should exceed 40 visible characters
