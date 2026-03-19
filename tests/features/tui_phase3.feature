@tui @pending
Feature: TUI Phase 3 — Markdown Rendering and Tool Output Display
  Rich content rendering for assistant messages and tool output.

  # ---------------------------------------------------------------------------
  # Markdown
  # ---------------------------------------------------------------------------

  Scenario: Markdown renders headings with styling
    Given a markdown component with text "# Hello\n## World"
    When the component renders at width 80
    Then the rendered output should contain styled heading "Hello"
    And the rendered output should contain styled heading "World"

  Scenario: Markdown renders bold and italic inline
    Given a markdown component with text "This is **bold** and *italic* text"
    When the component renders at width 80
    Then the rendered output should contain bold "bold"
    And the rendered output should contain italic "italic"

  Scenario: Markdown renders code blocks with borders
    Given a markdown component with text "```rust\nfn main() {}\n```"
    When the component renders at width 80
    Then the rendered output should contain "fn main()"
    And the rendered output should contain code block borders

  Scenario: Markdown renders inline code
    Given a markdown component with text "Use `cargo build` to compile"
    When the component renders at width 80
    Then the rendered output should contain styled code "cargo build"

  Scenario: Markdown renders lists
    Given a markdown component with text "- item one\n- item two\n- item three"
    When the component renders at width 80
    Then the rendered output should contain bullet markers
    And the rendered output should contain "item one"

  Scenario: Markdown renders blockquotes
    Given a markdown component with text "> This is a quote"
    When the component renders at width 80
    Then the rendered output should contain quote border
    And the rendered output should contain "This is a quote"

  Scenario: Markdown renders horizontal rules
    Given a markdown component with text "---"
    When the component renders at width 80
    Then the rendered output should contain a horizontal rule

  Scenario: Markdown renders links
    Given a markdown component with text "[Example](https://example.com)"
    When the component renders at width 80
    Then the rendered output should contain "Example"

  # ---------------------------------------------------------------------------
  # Tool Output
  # ---------------------------------------------------------------------------

  Scenario: Tool execution shows collapsible output
    Given a tool execution component for "bash" with result "file1.txt\nfile2.txt"
    When the component renders collapsed at width 80
    Then the rendered output should show the tool name
    And the result should be truncated

  Scenario: Tool execution shows expanded output
    Given a tool execution component for "bash" with result "file1.txt\nfile2.txt"
    When the component renders expanded at width 80
    Then the rendered output should contain "file1.txt"
    And the rendered output should contain "file2.txt"

  Scenario: Tool execution shows error styling
    Given a tool execution component for "bash" with error "command not found"
    When the component renders at width 80
    Then the rendered output should show error styling

  Scenario: Edit tool shows diff with colors
    Given a tool execution component for "edit" with diff "+added line\n-removed line"
    When the component renders expanded at width 80
    Then the rendered output should contain green-styled added line
    And the rendered output should contain red-styled removed line
