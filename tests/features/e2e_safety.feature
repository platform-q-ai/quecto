Feature: End-to-End Safety and Limits
  As a system operator
  I want sandbox enforcement, iteration limits, and wall-clock timeouts to work end-to-end
  So that the agent cannot escape its restrictions or run indefinitely

  Background:
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And restrict_to_workspace is true in the config

  # --- Sandbox enforcement through CLI ---

  @pending
  Scenario: Tool call to read file outside workspace returns error to LLM
    Given the mock LLM returns a tool call for "read_file" with args:
      | path | /etc/passwd |
    And the mock LLM then returns "I cannot access that file"
    When I run quecto agent -s - -m "Read /etc/passwd"
    Then the exit code should be 0
    And stdout should contain "cannot access"

  @pending
  Scenario: Tool call to execute dangerous command returns error to LLM
    Given the mock LLM returns a tool call for "exec" with args:
      | command | rm -rf / |
    And the mock LLM then returns "That command is not allowed"
    When I run quecto agent -s - -m "Delete everything"
    Then the exit code should be 0
    And stdout should contain "not allowed"

  @pending
  Scenario: Path traversal via tool call is blocked
    Given the mock LLM returns a tool call for "read_file" with args:
      | path | ../../etc/passwd |
    And the mock LLM then returns "Access denied"
    When I run quecto agent -s - -m "Read ../../etc/passwd"
    Then the exit code should be 0
    And stdout should contain "Access denied"

  # --- Iteration limits ---

  @pending
  Scenario: Agent stops after max-iterations flag
    Given the mock LLM always returns a tool call for "exec" with args:
      | command | echo loop |
    When I run quecto agent -s - --max-iterations 3 -m "Loop forever"
    Then the exit code should be 0
    And stdout should contain "iteration limit"

  @pending
  Scenario: Default iteration limit is applied from config
    Given the config sets max_tool_iterations to 5
    And the mock LLM always returns a tool call for "exec" with args:
      | command | echo loop |
    When I run quecto agent -s - -m "Loop forever"
    Then the exit code should be 0
    And stdout should contain "iteration limit"

  @pending
  Scenario: Max-iterations flag overrides config value
    Given the config sets max_tool_iterations to 25
    And the mock LLM always returns a tool call for "exec" with args:
      | command | echo loop |
    When I run quecto agent -s - --max-iterations 2 -m "Loop forever"
    Then the exit code should be 0
    And stdout should contain "iteration limit"

  # --- Wall-clock timeout ---

  @pending
  Scenario: Agent is killed after max-time exceeded
    Given the mock LLM takes 5 seconds to respond
    When I run quecto agent -s - --max-time 1 -m "Slow question"
    Then the exit code should be 2
    And stderr should contain "max-time exceeded"

  @pending
  Scenario: Agent completes within max-time
    Given the mock LLM returns a text response "Quick reply"
    When I run quecto agent -s - --max-time 10 -m "Fast question"
    Then the exit code should be 0
    And stdout should contain "Quick reply"

  @pending
  Scenario: Max-time covers total elapsed time including tool execution
    Given the mock LLM returns a tool call for "exec" with args:
      | command | sleep 3 |
    And the mock LLM then returns "Done"
    When I run quecto agent -s - --max-time 1 -m "Run slow command"
    Then the exit code should be 2
    And stderr should contain "max-time exceeded"
