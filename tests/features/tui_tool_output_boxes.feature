@tui @pending
Feature: TUI Pi-style tool output — colored boxes, expand/collapse
  Issue #473: Tool output renders as simple inline text. Should use boxed
  tool blocks with colored backgrounds, expand/collapse via Ctrl+O, and
  styled content previews.

  # ---------------------------------------------------------------------------
  # Box rendering
  # ---------------------------------------------------------------------------

  Scenario: Running tool shows box with pending color
    Given a chat component with tool_expanded false
    When a tool_start event arrives for "bash" with args {"command":"ls -la"}
    Then the rendered output should contain a top border character "─"
    And the rendered output should contain the tool name "bash"

  Scenario: Completed tool shows box with success indicator
    Given a chat component
    And a completed "bash" tool with result "file.txt"
    Then the rendered output should contain a success icon
    And the rendered output should contain "bash"

  Scenario: Failed tool shows box with error indicator
    Given a chat component
    And a failed "bash" tool with error "command not found"
    Then the rendered output should contain an error icon
    And the rendered output should contain the error text

  # ---------------------------------------------------------------------------
  # Expand / collapse
  # ---------------------------------------------------------------------------

  Scenario: Completed tool is collapsed by default
    Given a chat component with tool_expanded false
    And a completed "bash" tool with multi-line result
    Then the result preview should show only the first line

  Scenario: Ctrl+O expands all tool outputs
    Given a chat component with tool_expanded false
    And a completed "bash" tool with multi-line result
    When tool_expanded is set to true
    Then the result should show all output lines (up to limit)

  Scenario: Ctrl+O collapses expanded tool outputs
    Given a chat component with tool_expanded true
    And a completed "bash" tool with multi-line result
    When tool_expanded is set to false
    Then the result preview should show only the first line

  # ---------------------------------------------------------------------------
  # Tool-specific formatting
  # ---------------------------------------------------------------------------

  Scenario: Bash tool shows command in header
    Given a chat component
    When a bash tool completes with command "ls -la" and output "total 42"
    Then the header should contain "ls -la"

  Scenario: Read tool shows file path in header
    Given a chat component
    When a read tool completes with path "src/main.rs" and content "fn main()"
    Then the header should contain "src/main.rs"

  Scenario: Edit tool shows diff-colored output when expanded
    Given a chat component with tool_expanded true
    And an edit tool completes with diff "+added\n-removed\n context"
    Then added lines should be green
    And removed lines should be red

  # ---------------------------------------------------------------------------
  # Duration display
  # ---------------------------------------------------------------------------

  Scenario: Duration shown in tool header
    Given a chat component
    And a completed "bash" tool with duration 42ms
    Then the rendered output should contain "42ms"

  # ---------------------------------------------------------------------------
  # Box border
  # ---------------------------------------------------------------------------

  Scenario: Tool box has top border
    Given a chat component
    And a completed "bash" tool
    Then the rendered output should contain a top border line with "─" characters

  Scenario: Tool box has bottom border
    Given a chat component
    And a completed "bash" tool
    Then the rendered output should contain a bottom border line with "─" characters

  # ---------------------------------------------------------------------------
  # Width compliance
  # ---------------------------------------------------------------------------

  Scenario: Tool box respects terminal width
    Given a chat component with tool entries
    When rendered at width 40
    Then no rendered line should exceed 40 visible characters
