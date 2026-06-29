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
