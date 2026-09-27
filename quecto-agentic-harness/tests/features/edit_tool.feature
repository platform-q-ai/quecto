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

  # --- Refusals that say what to do next (#2193) ---

  @done
  Scenario: Every match of an ambiguous oldText is counted
    Given a tool workspace
    And a file "triple.txt" exists with content "triple triple triple\n"
    When the agent edits "triple.txt" replacing "triple" with "x"
    Then the [ToolResult] should be an error
    And the [ToolResult] should contain "oldText matches 3 times in triple.txt, all on line 1"
    And the [ToolResult] should contain "Add surrounding lines"

  @done
  Scenario: Editing a missing file points to write
    Given a tool workspace
    When the agent edits "missing.txt" replacing "a" with "b"
    Then the [ToolResult] should be an error
    And the [ToolResult] should contain "file not found: missing.txt"
    And the [ToolResult] should contain "use write to create a new one"

  @done
  Scenario: A binary file is refused as not UTF-8 text
    Given a tool workspace
    And a binary file "blob.bin" exists
    When the agent edits "blob.bin" replacing "a" with "b"
    Then the [ToolResult] should be an error
    And the [ToolResult] should contain "blob.bin is not UTF-8 text"
    And the [ToolResult] should contain "xxd"

  @done
  Scenario: oldText indented unlike the file gets an indentation hint
    Given a tool workspace
    And a file "indent.txt" exists with content "\tindented text\n"
    When the agent edits "indent.txt" replacing "    indented text" with "x"
    Then the [ToolResult] should be an error
    And the [ToolResult] should contain "matches at line 1 if indentation is ignored"
    And the [ToolResult] should contain "line 1 is indented with 1 tab in the file, 4 spaces in oldText"
    And the file "indent.txt" should read exactly "\tindented text\n"

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

  @done
  Scenario: Fuzzy match on a file with trailing spaces replaces only the matched lines
    Given a tool workspace
    And a file "pad.txt" exists with content "pad  \nfoo  \nbar\nend\n"
    When the agent edits "pad.txt" replacing "foo\nbar" with "X"
    Then the [ToolResult] should not be an error
    And the file "pad.txt" should read exactly "pad  \nX\nend\n"

  @done
  Scenario: Fuzzy match on a file with curly quotes before the match
    Given a tool workspace
    And a file "curly.txt" exists with content "’’ab’\n"
    When the agent edits "curly.txt" replacing "b'" with "Z"
    Then the [ToolResult] should not be an error
    And the file "curly.txt" should read exactly "’’aZ\n"

  @done
  Scenario: Fuzzy match with whitespace at the end of oldText replaces the file's whitespace
    Given a tool workspace
    And a file "edge.txt" exists with content "it’s foo bar\n"
    When the agent edits "edge.txt" replacing "it's foo " with "it's baz "
    Then the [ToolResult] should not be an error
    And the file "edge.txt" should read exactly "it's baz bar\n"

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

  @done
  Scenario: A change at the end of a 1 MiB line is shown in the diff
    Given a tool workspace
    And a file "large.txt" exists with 1048569 "a" characters then "ENDING!"
    When the agent edits "large.txt" replacing "ENDING!" with "FINISH!"
    Then the [ToolResult] should not be an error
    And the [ToolResult] should contain "aaaENDING!"
    And the [ToolResult] should contain "aaaFINISH!"
    And the [ToolResult] should contain "the change starts at column 1048570"
    And the tool result should be at most 600 bytes

  @done
  Scenario: A line added next to a changed long line does not hide the change
    Given a tool workspace
    And a file "long.txt" exists with 2000 "a" characters then "ENDING!"
    When the agent edits "long.txt" replacing "aaaaENDING!" with "aaaaFINISH!\n// footer"
    Then the [ToolResult] should not be an error
    And the [ToolResult] should contain "aaaENDING!"
    And the [ToolResult] should contain "aaaFINISH!"
    And the [ToolResult] should contain "// footer"
