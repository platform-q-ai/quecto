Feature: End-to-End Agentic Loop
  As a user running the agent CLI
  I want the agent to chain multiple tool calls, handle errors, and use all tool types
  So that complex multi-step tasks execute correctly through the full stack

  Background:
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server

  # --- Untested tool types ---

  @done
  Scenario: LLM edits a file via edit_file tool
    Given a file "config.txt" in the e2e workspace with content "mode=debug"
    And the mock LLM first returns a tool call for "edit_file" with args:
      | path | config.txt |
      | old  | debug      |
      | new  | release    |
    And the mock LLM then returns a text response "Changed mode to release"
    When I run quecto agent -s - -m "Switch config to release mode"
    Then the exit code should be 0
    And stdout should contain "Changed mode to release"
    And the file "config.txt" in the e2e workspace should contain "mode=release"

  @done
  Scenario: LLM appends to a file via append_file tool
    Given a file "log.txt" in the e2e workspace with content "line1"
    And the mock LLM first returns a tool call for "append_file" with args:
      | path    | log.txt      |
      | content | appended-line |
    And the mock LLM then returns a text response "Appended to log"
    When I run quecto agent -s - -m "Add a line to the log"
    Then the exit code should be 0
    And the file "log.txt" in the e2e workspace should contain "line1"
    And the file "log.txt" in the e2e workspace should contain "appended-line"

  @done
  Scenario: edit_file error when substring not found is sent back to LLM
    Given a file "data.txt" in the e2e workspace with content "hello world"
    And the mock LLM first returns a tool call for "edit_file" with args:
      | path | data.txt  |
      | old  | not-found |
      | new  | replaced  |
    And the mock LLM then returns a text response "The substring was not in the file"
    When I run quecto agent -s - -m "Edit data.txt"
    Then the exit code should be 0
    And stdout should contain "not in the file"
    And the file "data.txt" in the e2e workspace should contain "hello world"

  # --- Deep tool chaining (3+ calls) ---

  @done
  Scenario: Three-step pipeline reads, transforms via exec, and writes result
    Given a file "input.txt" in the e2e workspace with content "hello world"
    And the mock LLM returns a tool call sequence:
      | call | read_file  | {"path":"input.txt"}                         |
      | call | exec       | {"command":"echo HELLO WORLD"}                |
      | call | write | {"path":"output.txt","content":"HELLO WORLD"} |
      | text | Pipeline complete |                                        |
    When I run quecto agent -s - -m "Uppercase input.txt and save to output.txt"
    Then the exit code should be 0
    And the file "output.txt" should exist in the e2e workspace
    And the file "output.txt" in the e2e workspace should contain "HELLO WORLD"

  @done
  Scenario: Read-edit-read cycle verifies file was modified
    Given a file "version.txt" in the e2e workspace with content "v1.0.0"
    And the mock LLM returns a tool call sequence:
      | call | read_file | {"path":"version.txt"}                             |
      | call | edit_file | {"path":"version.txt","old":"1.0.0","new":"2.0.0"} |
      | call | read_file | {"path":"version.txt"}                             |
      | text | Version bumped from v1.0.0 to v2.0.0 |                        |
    When I run quecto agent -s - -m "Bump the version to 2.0.0"
    Then the exit code should be 0
    And stdout should contain "bumped"
    And the file "version.txt" in the e2e workspace should contain "v2.0.0"

  # --- Parallel tool calls ---

  @done
  Scenario: LLM issues two tool calls in a single response
    Given a file "a.txt" in the e2e workspace with content "alpha"
    And a file "b.txt" in the e2e workspace with content "beta"
    And the mock LLM returns parallel tool calls then text:
      | read_file | {"path":"a.txt"} | read_file | {"path":"b.txt"} |
    And the final text is "Files contain: alpha and beta"
    When I run quecto agent -s - -m "Read both files"
    Then the exit code should be 0
    And stdout should contain "alpha and beta"

  # --- Error recovery with corrective action ---

  @done
  Scenario: LLM recovers from a missing file by creating it
    Given the mock LLM returns a tool call sequence:
      | call | read_file  | {"path":"missing.txt"}                    |
      | call | write | {"path":"missing.txt","content":"created"} |
      | call | read_file  | {"path":"missing.txt"}                    |
      | text | File created and verified |                             |
    When I run quecto agent -s - -m "Read missing.txt or create it"
    Then the exit code should be 0
    And stdout should contain "created and verified"
    And the file "missing.txt" should exist in the e2e workspace
    And the file "missing.txt" in the e2e workspace should contain "created"

  # --- Exec output drives next decision ---

  @done
  Scenario: Exec output is available to LLM for subsequent tool call
    Given a file "target.txt" in the e2e workspace with content "secret data"
    And the mock LLM returns a tool call sequence:
      | call | exec      | {"command":"ls"}      |
      | call | read_file | {"path":"target.txt"} |
      | text | Found file and read: secret data | |
    When I run quecto agent -s - -m "List files then read the interesting one"
    Then the exit code should be 0
    And stdout should contain "secret data"
