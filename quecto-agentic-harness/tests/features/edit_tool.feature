@done
Feature: EditTool — Quecto compatibility
  As an LLM agent
  I want the edit tool to handle LLM-emitted Unicode quirks and file format variations
  So that edits succeed without manual retry loops

  # --- Existing behaviour (regression guard) ---

  @done
  Scenario: Exact match replaces content
    Given a tool workspace
    And a file "code.py" exists with content "print('hello')"
    When the agent executes tool "edit" with args:
      | path    | code.py |
      | oldText | hello   |
      | newText | world   |
    Then the file "code.py" should contain "print('world')"
    And the [ToolResult] should not be an error

  @done
  Scenario: oldText not found returns error
    Given a tool workspace
    And a file "f.txt" exists with content "hello world"
    When the agent executes tool "edit" with args:
      | path    | f.txt |
      | oldText | xyz   |
      | newText | abc   |
    Then the [ToolResult] should be an error
    And the [ToolResult] should contain "not found"

  @done
  Scenario: Ambiguous match returns error
    Given a tool workspace
    And a file "dup.txt" exists with content "x = 1\nx = 1"
    When the agent executes tool "edit" with args:
      | path    | dup.txt |
      | oldText | x = 1   |
      | newText | x = 2   |
    Then the [ToolResult] should be an error
    And the [ToolResult] should contain "matches"

  # --- Fuzzy content matching ---

  @done
  Scenario: Fuzzy match normalises smart single quotes in oldText
    Given a tool workspace
    And a file "quotes.txt" exists with content "it's a test"
    When the agent executes tool "edit" with smart-single-quote oldText on "quotes.txt"
    Then the file "quotes.txt" should contain "it's replaced"
    And the [ToolResult] should not be an error

  @done
  Scenario: Fuzzy match normalises smart double quotes in oldText
    Given a tool workspace
    And a file "dquotes.txt" exists with content "say \"hello\" now"
    When the agent executes tool "edit" with smart-double-quote oldText on "dquotes.txt"
    Then the file "dquotes.txt" should contain "say \"goodbye\" now"
    And the [ToolResult] should not be an error

  @done
  Scenario: Fuzzy match normalises Unicode en-dash in oldText
    Given a tool workspace
    And a file "dash.txt" exists with content "hello - world"
    When the agent executes tool "edit" with en-dash oldText on "dash.txt"
    Then the file "dash.txt" should contain "replaced"
    And the [ToolResult] should not be an error

  @done
  Scenario: Fuzzy match normalises trailing whitespace per line
    Given a tool workspace
    And a file "spaces.txt" exists with content "hello\nworld"
    When the agent executes tool "edit" with trailing-whitespace oldText on "spaces.txt"
    Then the file "spaces.txt" should contain "replaced"
    And the [ToolResult] should not be an error

  # --- Line-ending preservation ---

  @done
  Scenario: CRLF line endings preserved after edit
    Given a tool workspace
    And a file "win.txt" exists with CRLF bytes "line1\r\nline2\r\nline3\r\n"
    When the agent executes tool "edit" with args:
      | path    | win.txt |
      | oldText | line2   |
      | newText | EDITED  |
    Then the file "win.txt" should contain CRLF line endings
    And the file "win.txt" should contain "EDITED"
    And the [ToolResult] should not be an error

  @done
  Scenario: LF line endings preserved after edit
    Given a tool workspace
    And a file "unix.txt" exists with content "line1\nline2\nline3\n"
    When the agent executes tool "edit" with args:
      | path    | unix.txt |
      | oldText | line2    |
      | newText | EDITED   |
    Then the file "unix.txt" should not contain CRLF line endings
    And the file "unix.txt" should contain "EDITED"
    And the [ToolResult] should not be an error

  # --- BOM preservation ---

  @done
  Scenario: BOM preserved after edit
    Given a tool workspace
    And a file "bom.txt" exists with UTF-8 BOM and content "hello world"
    When the agent executes tool "edit" with args:
      | path    | bom.txt |
      | oldText | hello   |
      | newText | hi      |
    Then the file "bom.txt" should start with a UTF-8 BOM
    And the file "bom.txt" should contain "hi world"
    And the [ToolResult] should not be an error

  # --- No-op detection ---

  @done
  Scenario: No-op replacement detected and rejected
    Given a tool workspace
    And a file "noop.txt" exists with content "hello world"
    When the agent executes tool "edit" with args:
      | path    | noop.txt    |
      | oldText | hello world |
      | newText | hello world |
    Then the [ToolResult] should be an error
    And the [ToolResult] should contain "identical"

  # --- Improved diff output ---

  @done
  Scenario: Diff output shows changed lines with line numbers
    Given a tool workspace
    And a file "multi.txt" exists with content "line1\nline2\nline3\nline4\nline5"
    When the agent executes tool "edit" with args:
      | path    | multi.txt |
      | oldText | line3     |
      | newText | CHANGED   |
    Then the [ToolResult] should contain "-3 line3"
    And the [ToolResult] should contain "+3 CHANGED"
    And the [ToolResult] should not be an error

  @done
  Scenario: Diff context includes 4 surrounding lines
    Given a tool workspace
    And a file "ctx.txt" exists with content "a\nb\nc\nd\ne\nf\ng\nh\ni\nj"
    When the agent executes tool "edit" with args:
      | path    | ctx.txt |
      | oldText | f       |
      | newText | F       |
    Then the [ToolResult] should contain "b"
    And the [ToolResult] should contain "c"
    And the [ToolResult] should contain "d"
    And the [ToolResult] should contain "e"
    And the [ToolResult] should not be an error

  @done
  Scenario: Over-cap diff output is bounded but still verifiable
    Given a tool workspace
    And a large over-cap diff fixture "large-diff.txt" exists
    When the agent executes an over-cap edit on "large-diff.txt"
    Then the [ToolResult] should contain "Successfully edited large-diff.txt"
    And the [ToolResult] should contain "-  1 old line 000"
    And the [ToolResult] should contain "-  2 old line 001"
    And the [ToolResult] should contain "[diff truncated:"
    And the [ToolResult] should contain "hunks shown"
    And the [ToolResult] should contain "lines changed total"
    And the tool result should be at most 4096 bytes
    And the [ToolResult] should not be an error
