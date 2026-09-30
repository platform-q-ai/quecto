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
  Scenario: A POSIX SHELL gives way to bash, which the tool is named for
    When the agent executes bash "echo $0" with shell env "sh"
    Then the [ToolResult] should not be an error
    And the [ToolResult] should contain "bash"

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
  Scenario: A long line within the 50KB budget comes back whole (#2196)
    When the agent executes tool "bash" with args:
      | command | printf 'j%.0s' {1..15000} |
    Then the [ToolResult] should not be an error
    And the [ToolResult] should not contain "omitted"
    And the [ToolResult] should not contain "Showing lines"

  @done
  Scenario: An over-long single line shows its start and end, and says so (#2196)
    When the agent executes tool "bash" with args:
      | command | printf 'b%.0s' {1..60000} |
    Then the [ToolResult] should not be an error
    And the [ToolResult] should contain "bytes of line 1 omitted"
    And the [ToolResult] should contain "Showing lines 1-1 of 1 (50KB limit); line 1 is 60000 bytes, of which the first and last 2KB are shown"
    And the [ToolResult] should contain "`read` the saved file for lines up to 50KB whole"
    And the [ToolResult] should contain "Full output (60000 bytes) saved to:"
    And the [ToolResult] should be shorter than 6000 characters

  # --- Binary output ---

  @done
  Scenario: Binary output is named with its size, not decoded (#2197)
    When the agent executes tool "bash" with args:
      | command | head -c 500 /dev/urandom |
    Then the [ToolResult] should not be an error
    And the [ToolResult] should contain "[binary output on stdout (500 bytes, not UTF-8 text) not shown."
    And the [ToolResult] should contain "od -c"
    And the [ToolResult] should not contain "�"

  @done
  Scenario: Text with a stray byte stays text (#2197)
    When the agent executes tool "bash" with args:
      | command | printf 'caf\351 menu\n' |
    Then the [ToolResult] should not be an error
    And the [ToolResult] should contain "caf� menu"
    And the [ToolResult] should not contain "binary output"

  @done
  Scenario: NUL-separated names stay text (#2197)
    When the agent executes tool "bash" with args:
      | command | printf './a\0./b\0' |
    Then the [ToolResult] should not be an error
    And the [ToolResult] should contain "./b"
    And the [ToolResult] should not contain "binary output"

  # --- Command policy ---

  @done
  Scenario: A command-policy refusal says why and what to do instead (#2198)
    When the agent executes tool "bash" with args:
      | command | /bin/ech? hi |
    Then the tool call should fail with "blocked by command policy (rule glob-command-name)"
    And the tool call should fail with "name the program literally"
    And the tool call should fail with "checked again"
    And the tool call should fail with "tell the user"

  @done
  Scenario: A blanket refusal says no other way is allowed (#2198)
    When the agent executes tool "bash" with args:
      | command | mkfs --version |
    Then the tool call should fail with "blocked by command policy (rule mkfs)"
    And the tool call should fail with "do not run here in any form"
    And the tool call should fail with "Getting the same effect another way is not allowed; if the task needs it, tell the user."

  @done
  Scenario: output_file writes full combined output and returns a summary
    When the agent executes bash with output_file "snapshots/out.txt" and command "printf 'out\\n'; printf 'err\\n' >&2; exit 7"
    Then the [ToolResult] should be an error
    And the [ToolResult] should contain "exit code 7"
    And the [ToolResult] should contain "output saved to:"
    And bash output_file "snapshots/out.txt" should contain "out\nerr\n"

  @done
  Scenario: output_file keeps large output out of the inline result
    When the agent executes bash with output_file "large.txt" and command "head -c 12000000 /dev/zero | tr -c A A; echo"
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

  @done
  Scenario: invalid output_file path returns a tool error
    When the agent executes bash with output_file "." and command "printf 'not-written'"
    Then the tool call should fail with "bash output_file failed"
