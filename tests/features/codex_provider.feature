@done @providers
Feature: Codex Responses API Provider
  As a user using GPT-5.3-codex via OAuth
  I want the Codex provider to correctly format Responses API requests
  So that tool calls work even when reasoning items precede function calls

  # --- Request body formation ---

  Scenario: Codex request body includes tool_choice auto
    Given a Codex request body for model "gpt-5.3-codex" with tools
    Then the request body should contain "tool_choice" set to "auto"

  Scenario: Codex request body includes parallel_tool_calls
    Given a Codex request body for model "gpt-5.3-codex" with tools
    Then the request body should contain "parallel_tool_calls" set to true

  Scenario: Codex request body includes reasoning configuration
    Given a Codex request body for model "gpt-5.3-codex" with tools
    Then the request body should contain a "reasoning" object with "effort" set to "medium"
    And the request body should contain a "reasoning" object with "summary" set to "auto"

  Scenario: Codex request body includes reasoning encrypted content
    Given a Codex request body for model "gpt-5.3-codex" with tools
    Then the request body should contain "include" with "reasoning.encrypted_content"

  Scenario: Codex request body includes text verbosity
    Given a Codex request body for model "gpt-5.3-codex" with tools
    Then the request body should contain a "text" object with "verbosity" set to "medium"

  Scenario: Codex request body does not include max_completion_tokens
    Given a Codex request body for model "gpt-5.3-codex" with tools
    Then the request body should not contain "max_completion_tokens"

  Scenario: Codex tool definitions include strict false
    Given a Codex request body for model "gpt-5.3-codex" with tools
    Then each tool definition should have "strict" set to false

  # --- SSE parsing with reasoning items ---

  Scenario: Codex SSE parser handles tool calls after reasoning items
    Given a Codex SSE stream with a reasoning item at output_index 0 and a function call at output_index 1
    When I parse the Codex SSE stream
    Then the parsed response should have 1 tool call
    And the tool call should have name "exec"
    And the tool call should have arguments containing "ls"

  Scenario: Codex SSE parser handles multiple tool calls after reasoning
    Given a Codex SSE stream with a reasoning item at output_index 0 and function calls at output_index 1 and 2
    When I parse the Codex SSE stream
    Then the parsed response should have 2 tool calls
    And tool call 0 should have name "read" and arguments containing "main.rs"
    And tool call 1 should have name "exec" and arguments containing "cargo"

  Scenario: Codex SSE parser handles tool calls without reasoning items
    Given a Codex SSE stream with function calls at output_index 0 and 1 without reasoning
    When I parse the Codex SSE stream
    Then the parsed response should have 2 tool calls
    And tool call 0 should have name "read" and arguments containing "file.txt"
    And tool call 1 should have name "write" and arguments containing "output"
