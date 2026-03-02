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
    And the tool call should have name "bash"
    And the tool call should have arguments containing "ls"

  Scenario: Codex SSE parser handles multiple tool calls after reasoning
    Given a Codex SSE stream with a reasoning item at output_index 0 and function calls at output_index 1 and 2
    When I parse the Codex SSE stream
    Then the parsed response should have 2 tool calls
    And tool call 0 should have name "read" and arguments containing "main.rs"
    And tool call 1 should have name "bash" and arguments containing "cargo"

  Scenario: Codex SSE parser handles tool calls without reasoning items
    Given a Codex SSE stream with function calls at output_index 0 and 1 without reasoning
    When I parse the Codex SSE stream
    Then the parsed response should have 2 tool calls
    And tool call 0 should have name "read" and arguments containing "file.txt"
    And tool call 1 should have name "write" and arguments containing "output"

  # --- prompt_cache_key (session-based prompt caching) ---

  Scenario: Codex request body includes prompt_cache_key when session ID is set
    Given a Codex request body for model "gpt-5.3-codex" with session ID "cli:default"
    Then the request body should contain a sanitized "prompt_cache_key" with prefix "cli"

  Scenario: Codex request body omits prompt_cache_key when no session ID is set
    Given a Codex request body for model "gpt-5.3-codex" without a session ID
    Then the request body should not contain "prompt_cache_key"

  # --- Issue #192: orphaned function_call/function_call_output repair ---

  Scenario: Orphaned function_call without output is removed from input
    Given a message list with an assistant function_call "call_orphan" but no matching output
    When I build the Codex input
    Then the input should not contain any item with call_id "call_orphan"

  Scenario: Orphaned function_call_output without call is removed from input
    Given a message list with a tool result for "call_orphan" but no matching function_call
    When I build the Codex input
    Then the input should not contain any item with call_id "call_orphan"

  Scenario: Valid matched function_call and output pairs are preserved
    Given a message list with a matched function_call "call_valid" and its output
    When I build the Codex input
    Then the input should contain an item with call_id "call_valid" of type "function_call"
    And the input should contain an item with call_id "call_valid" of type "function_call_output"

  Scenario: Mixed valid and orphaned pairs — only orphans are removed
    Given a message list with a matched pair "call_good" and an orphaned function_call "call_bad"
    When I build the Codex input
    Then the input should contain an item with call_id "call_good" of type "function_call"
    And the input should contain an item with call_id "call_good" of type "function_call_output"
    And the input should not contain any item with call_id "call_bad"
