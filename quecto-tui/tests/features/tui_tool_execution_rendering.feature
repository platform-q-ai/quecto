@tui @done @tool-render
Feature: TUI tool execution rendering
  As a TUI operator
  I want tool executions to render concise status, previews, and expansion hints
  So that I can understand tool progress and inspect output without losing context

  Scenario: Bash output is collapsed to the latest lines until expanded
    Given a fresh TUI tool rendering harness
    When a bash tool call runs command "printf many" with 8 output lines
    Then the tool rendering shows "$ printf many"
    And the tool rendering shows "line-8"
    And the tool rendering hides "line-1"
    And the tool rendering shows "3 earlier lines, Ctrl+O to expand"
    When I expand tool output in the TUI
    Then the tool rendering shows "line-1"
    And the tool rendering hides "earlier lines, Ctrl+O to expand"

  Scenario: Read previews sanitize terminal controls from tool output
    Given a fresh TUI tool rendering harness
    When a read tool call previews path "src/lib.rs" with controlled content
    Then the tool rendering shows "read src/lib.rs"
    And the tool rendering shows "safevalue"
    And the tool rendering hides "pwned-title"
    And the raw tool frame does not contain terminal title escape controls

  Scenario: Workflow tool calls render action summaries and first result line
    Given a fresh TUI tool rendering harness
    When a workflow tool call checks step 2 with multiline result
    Then the tool rendering shows "workflow check step 2"
    And the tool rendering shows "Step 2 checked."
    And the tool rendering hides "extra detail"
