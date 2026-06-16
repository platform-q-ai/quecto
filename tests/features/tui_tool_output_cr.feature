Feature: Tool output carriage return stripping (#529)
  As a TUI user running git commands
  I want carriage return characters stripped from tool output
  So that the display renders cleanly without black line artefacts

  # All rendering verification is done via unit tests in
  # quecto-tui/src/interface/components/tool_output.rs (strip_carriage_returns tests).
  # These BDD scenarios verify the protocol-level content handling.

  @wip
  Scenario: Tool result content with CRLF is normalized
    Given a tool execution result with content "line1\r\nline2\r\n"
    When the content is processed for display
    Then the processed content should be "line1\nline2\n"

  @wip
  Scenario: Bare carriage returns from progress output are collapsed
    Given a tool execution result with content "Working...\rDone!\n"
    When the content is processed for display
    Then the processed content should be "Done!\n"

  @wip
  Scenario: Normal content without CR is unaffected
    Given a tool execution result with content "file1.txt\nfile2.txt\nfile3.txt"
    When the content is processed for display
    Then the processed content should be "file1.txt\nfile2.txt\nfile3.txt"
