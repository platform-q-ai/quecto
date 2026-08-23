Feature: ExecTool (bash) — Quecto compatibility
  As an AI agent
  I want the bash tool to match Quecto's feature set
  So that long-running commands work and truncation notices are informative

  Background:
    Given a tool workspace

  # --- Per-invocation timeout parameter ---

  @done
  Scenario: Per-invocation timeout terminates a slow command
    When the agent executes tool "bash" with args:
      | command | sleep 10   |
      | timeout | 1          |
    Then the [ToolResult] should be an error
    And the [ToolResult] should contain "timed out"

  @done
  Scenario: Per-invocation timeout cannot exceed configured maximum
    When the agent executes tool "bash" with args:
      | command | echo hi    |
      | timeout | 99999      |
    Then the [ToolResult] should not be an error

  @done
  Scenario: Default timeout applies when parameter omitted
    When the agent executes tool "bash" with args:
      | command | echo hello |
    Then the [ToolResult] should not be an error
    And the [ToolResult] should contain "hello"

  # --- Shell detection ---

  @done
  Scenario: Shell detection uses SHELL env variable
    When the agent executes bash "echo $0" with shell env "sh"
    Then the [ToolResult] should not be an error
    And the [ToolResult] should contain "sh"

  # --- commandPrefix option ---

  @done
  Scenario: commandPrefix is prepended to every command
    When the agent executes bash with command prefix "export MY_PREFIX=1" and command "echo $MY_PREFIX"
    Then the [ToolResult] should not be an error
    And the [ToolResult] should contain "1"

  # --- Truncation notice format ---

  @done
  Scenario: Byte-truncated output notice shows line range and 50KB limit
    Given a large output command that produces 60000 bytes
    When the agent executes that command via the bash tool
    Then the [ToolResult] should contain "Showing lines"
    And the [ToolResult] should contain "50KB limit"

  @done
  Scenario: Line-truncated output notice shows line range
    Given a large output command that produces 2100 lines
    When the agent executes that command via the bash tool
    Then the [ToolResult] should contain "Showing lines"
    And the [ToolResult] should not contain "50KB limit"

  @done
  Scenario: output_file writes full combined output and returns a summary
    When the agent executes bash with output_file "snapshots/out.txt" and command "printf 'out\\n'; printf 'err\\n' >&2; exit 7"
    Then the [ToolResult] should be an error
    And the [ToolResult] should contain "exit code 7"
    And the [ToolResult] should contain "output saved to:"
    And bash output_file "snapshots/out.txt" should contain "out\nerr\n"

  @done
  Scenario: output_file keeps large output out of the inline result
    When the agent executes bash with output_file "large.txt" and command "python3 - <<'PY'\nprint('A' * 12000000)\nPY"
    Then the [ToolResult] should not be an error
    And the [ToolResult] should contain "bytes:"
    And the [ToolResult] should be shorter than 4096 characters
    And bash output_file "large.txt" should contain 12000000 "A" characters

  @done
  Scenario: timeout returns captured tail
    When the agent executes bash with timeout 1 and command "printf 'before-timeout\\n'; sleep 60"
    Then the [ToolResult] should be an error
    And the [ToolResult] should contain "timed out"
    And the [ToolResult] should contain "before-timeout"

  @done
  Scenario: timeout with output_file marks saved output incomplete
    When the agent executes bash with timeout 1 output_file "timeout.txt" and command "printf 'before-timeout\\n'; sleep 60"
    Then the [ToolResult] should be an error
    And the [ToolResult] should contain "may be incomplete"
    And bash output_file "timeout.txt" should contain "before-timeout\n"
