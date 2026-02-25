@done
Feature: End-to-End Safety and Limits
  As a system operator
  I want sandbox enforcement, iteration limits, and wall-clock timeouts to work end-to-end
  So that the agent cannot escape its restrictions or run indefinitely

  Background:
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And restrict_to_workspace is enabled in the config

  # --- Sandbox enforcement through CLI ---

  Scenario: Tool call to read file outside workspace returns error to LLM
    Given the mock LLM first returns a tool call for "read_file" with args:
      | path | /etc/passwd |
    And the mock LLM then returns a text response "I cannot access that file"
    When I run quecto agent -s - -m "Read /etc/passwd"
    Then the exit code should be 0
    And stdout should contain "cannot access"

  Scenario: Tool call to execute dangerous command returns error to LLM
    Given the mock LLM first returns a tool call for "exec" with args:
      | command | rm -rf / |
    And the mock LLM then returns a text response "That command is not allowed"
    When I run quecto agent -s - -m "Delete everything"
    Then the exit code should be 0
    And stdout should contain "not allowed"

  Scenario: Tool call to execute wildcard root delete is blocked
    Given the mock LLM first returns a tool call for "exec" with args:
      | command | rm -rf /* |
    And the mock LLM then returns a text response "That destructive command is blocked"
    When I run quecto agent -s - -m "Delete root recursively"
    Then the exit code should be 0
    And stdout should contain "blocked"

  Scenario: Tool call to execute mkfs variant is blocked
    Given the mock LLM first returns a tool call for "exec" with args:
      | command | mkfs.ext4 /dev/sda |
    And the mock LLM then returns a text response "Formatting commands are blocked"
    When I run quecto agent -s - -m "Format the disk"
    Then the exit code should be 0
    And stdout should contain "blocked"

  Scenario: Path traversal via tool call is blocked
    Given the mock LLM first returns a tool call for "read_file" with args:
      | path | ../../etc/passwd |
    And the mock LLM then returns a text response "Access denied"
    When I run quecto agent -s - -m "Read ../../etc/passwd"
    Then the exit code should be 0
    And stdout should contain "Access denied"

  Scenario: Write_file outside workspace is blocked
    Given the mock LLM first returns a tool call for "write_file" with args:
      | path    | ../../tmp/pwned.txt |
      | content | owned               |
    And the mock LLM then returns a text response "Write denied"
    When I run quecto agent -s - -m "Write outside workspace"
    Then the exit code should be 0
    And stdout should contain "Write denied"

  Scenario: Edit_file outside workspace is blocked
    Given the mock LLM first returns a tool call for "edit_file" with args:
      | path | ../../etc/passwd |
      | old  | root             |
      | new  | pwned            |
    And the mock LLM then returns a text response "Edit denied"
    When I run quecto agent -s - -m "Edit protected file"
    Then the exit code should be 0
    And stdout should contain "Edit denied"

  Scenario: Append_file outside workspace is blocked
    Given the mock LLM first returns a tool call for "append_file" with args:
      | path    | ../../var/log/syslog |
      | content | injected             |
    And the mock LLM then returns a text response "Append denied"
    When I run quecto agent -s - -m "Append outside workspace"
    Then the exit code should be 0
    And stdout should contain "Append denied"

  Scenario: List_dir outside workspace is blocked
    Given the mock LLM first returns a tool call for "list_dir" with args:
      | path | ../../ |
    And the mock LLM then returns a text response "Listing denied"
    When I run quecto agent -s - -m "List parent directories"
    Then the exit code should be 0
    And stdout should contain "Listing denied"

  # --- Iteration limits ---

  Scenario: Agent stops after max-iterations flag
    Given the mock LLM always returns a tool call for "exec" with args:
      | command | echo loop |
    When I run quecto agent -s - --max-iterations 3 -m "Loop forever"
    Then the exit code should be 0
    And stdout should contain "iteration limit"

  Scenario: Default iteration limit is applied from config
    Given the config sets max_tool_iterations to 5
    And the mock LLM always returns a tool call for "exec" with args:
      | command | echo loop |
    When I run quecto agent -s - -m "Loop forever"
    Then the exit code should be 0
    And stdout should contain "iteration limit"

  Scenario: Max-iterations flag overrides config value
    Given the config sets max_tool_iterations to 25
    And the mock LLM always returns a tool call for "exec" with args:
      | command | echo loop |
    When I run quecto agent -s - --max-iterations 2 -m "Loop forever"
    Then the exit code should be 0
    And stdout should contain "iteration limit"

  # --- Wall-clock timeout ---

  Scenario: Agent is killed after max-time exceeded
    Given the mock LLM takes 5 seconds to respond
    When I run quecto agent -s - --max-time 1 -m "Slow question"
    Then the exit code should be 2
    And stderr should contain "max-time exceeded"

  Scenario: Agent completes within max-time
    Given the mock LLM returns a text response "Quick reply"
    When I run quecto agent -s - --max-time 10 -m "Fast question"
    Then the exit code should be 0
    And stdout should contain "Quick reply"

  Scenario: Max-time covers total elapsed time including tool execution
    Given exec isolation is set to native in the config
    And the mock LLM first returns a tool call for "exec" with args:
      | command | sleep 3 |
    And the mock LLM then returns a text response "Done"
    When I run quecto agent -s - --max-time 1 -m "Run slow command"
    Then the exit code should be 2
    And stderr should contain "max-time exceeded"
