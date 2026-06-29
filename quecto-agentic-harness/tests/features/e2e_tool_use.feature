Feature: End-to-End Tool Execution
  As a user running the agent CLI
  I want the LLM to invoke tools and incorporate their results
  So that the agent can interact with the filesystem and system on my behalf

  Background:
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server

  # --- Single tool call ---

  @done
  Scenario: LLM reads a file via tool call
    Given a file "notes.txt" in the e2e workspace with content "buy milk"
    And the mock LLM first returns a tool call for "read" with args:
      | path | notes.txt |
    And the mock LLM then returns a text response "Your notes say: buy milk"
    When I run quecto agent -s - -m "What are my notes?"
    Then the exit code should be 0
    And stdout should contain "Your notes say: buy milk"

  @done
  Scenario: LLM writes a file via tool call
    Given the mock LLM first returns a tool call for "write" with args:
      | path    | output.txt  |
      | content | hello world |
    And the mock LLM then returns a text response "File written successfully"
    When I run quecto agent -s - -m "Write hello world to output.txt"
    Then the exit code should be 0
    And the file "output.txt" should exist in the e2e workspace
    And the file "output.txt" in the e2e workspace should contain "hello world"

  @done
  Scenario: LLM executes a shell command via tool call
    Given the mock LLM first returns a tool call for "bash" with args:
      | command | echo hello from shell |
    And the mock LLM then returns a text response "The command output: hello from shell"
    When I run quecto agent -s - -m "Run echo hello"
    Then the exit code should be 0
    And stdout should contain "hello from shell"

  @done @e2e-tool-use
  Scenario: LLM exec tool with large output completes within max-time
    Given the mock LLM first returns a tool call for "bash" with args:
      | command | printf 'x%.0s' {1..100000} |
    And the mock LLM then returns a text response "Large output command completed"
    When I run quecto agent -s - --max-time 4 -m "Run a large output command"
    Then the exit code should be 0
    And stdout should contain "Large output command completed"

  @done
  Scenario: LLM lists a directory via tool call
    Given a file "a.txt" in the e2e workspace with content "a"
    And a file "b.txt" in the e2e workspace with content "b"
    And the mock LLM first returns a tool call for "ls" with args:
      | path | . |
    And the mock LLM then returns a text response "Directory contains: a.txt, b.txt"
    When I run quecto agent -s - -m "List the workspace"
    Then the exit code should be 0
    And stdout should contain "a.txt"

  # --- Multi-turn tool use ---

  @done
  Scenario: LLM makes two sequential tool calls
    Given a file "source.txt" in the e2e workspace with content "important data"
    And the mock LLM returns a tool call sequence:
      | call | read  | {"path":"source.txt"}                    |
      | call | write | {"path":"copy.txt","content":"important data"} |
      | text | Copied source.txt to copy.txt |                          |
    When I run quecto agent -s - -m "Copy source.txt to copy.txt"
    Then the exit code should be 0
    And the file "copy.txt" should exist in the e2e workspace
    And the file "copy.txt" in the e2e workspace should contain "important data"

  @done
  Scenario: Tool error is sent back to LLM and it recovers
    Given the mock LLM first returns a tool call for "read" with args:
      | path | nonexistent.txt |
    And the mock LLM then returns a text response "Sorry, that file does not exist"
    When I run quecto agent -s - -m "Read nonexistent.txt"
    Then the exit code should be 0
    And stdout should contain "does not exist"

  # --- Tool results in session ---

  @done
  Scenario: Tool call and result are included in persisted session
    Given a file "data.txt" in the e2e workspace with content "42"
    And the mock LLM first returns a tool call for "read" with args:
      | path | data.txt |
    And the mock LLM then returns a text response "The data is 42"
    When I run quecto agent -s tool-session -m "Read data.txt"
    Then the exit code should be 0
    And a [session] file should exist for key "cli:tool-session"
    And the [session] "cli:tool-session" should contain at least 4 messages
