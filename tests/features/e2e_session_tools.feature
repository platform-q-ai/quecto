@done
Feature: End-to-End Session + Tool Interactions
  As a user running multi-message agent sessions with tool use
  I want tool calls and results to be persisted in the session
  So that conversation history faithfully records what the agent did

  Background:
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server

  Scenario: Tool call and result are persisted with correct roles
    Given a file "info.txt" in the e2e workspace with content "secret=42"
    And the mock LLM first returns a tool call for "read_file" with args:
      | path | info.txt |
    And the mock LLM then returns a text response "The secret is 42"
    When I run quecto agent -s toolsess -m "Read the secret"
    Then the exit code should be 0
    And the session "cli:toolsess" should contain a tool role message
    And the session "cli:toolsess" should contain text "secret=42"

  Scenario: Second message in session sees tool history from first
    Given a file "data.txt" in the e2e workspace with content "alpha"
    And the mock LLM first returns a tool call for "read_file" with args:
      | path | data.txt |
    And the mock LLM then returns a text response "File says alpha"
    When I run quecto agent -s multi -m "Read data.txt"
    Then the exit code should be 0
    Given the mock LLM returns a text response "I remember alpha from the tool result"
    When I run quecto agent -s multi -m "What did data.txt contain?"
    Then the exit code should be 0
    And the session "cli:multi" should contain at least 6 messages
    And the session "cli:multi" should contain text "alpha"

  Scenario: Session with tool call history is loadable by subsequent run
    Given a pre-existing session "cli:resume" with tool call history for "read_file"
    And the mock LLM returns a text response "Resuming after tool use"
    When I run quecto agent -s resume -m "Continue"
    Then the exit code should be 0
    And stdout should contain "Resuming after tool use"
    And the session "cli:resume" should contain at least 6 messages

  Scenario: Multiple tool calls across messages accumulate in session
    Given the mock LLM first returns a tool call for "write" with args:
      | path    | first.txt |
      | content | one       |
    And the mock LLM then returns a text response "Wrote first.txt"
    When I run quecto agent -s accum -m "Create first.txt"
    Then the exit code should be 0
    Given the mock LLM first returns a tool call for "write" with args:
      | path    | second.txt |
      | content | two        |
    And the mock LLM then returns a text response "Wrote second.txt"
    When I run quecto agent -s accum -m "Create second.txt"
    Then the exit code should be 0
    And the session "cli:accum" should contain at least 8 messages
    And the file "first.txt" should exist in the e2e workspace
    And the file "second.txt" should exist in the e2e workspace

  Scenario: Ephemeral session with tool use leaves no trace
    Given the mock LLM first returns a tool call for "write" with args:
      | path    | ephemeral.txt |
      | content | temp data     |
    And the mock LLM then returns a text response "File created"
    When I run quecto agent -s - -m "Create ephemeral.txt"
    Then the exit code should be 0
    And the file "ephemeral.txt" should exist in the e2e workspace
    And no session files should exist

  Scenario: Tool error in first message does not corrupt session for second
    Given the mock LLM first returns a tool call for "read_file" with args:
      | path | missing.txt |
    And the mock LLM then returns a text response "File not found"
    When I run quecto agent -s errsess -m "Read missing.txt"
    Then the exit code should be 0
    Given the mock LLM returns a text response "All good now"
    When I run quecto agent -s errsess -m "How are things?"
    Then the exit code should be 0
    And stdout should contain "All good now"
    And the session "cli:errsess" should contain at least 6 messages
