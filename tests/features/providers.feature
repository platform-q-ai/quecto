@done @providers
Feature: LLM Providers
  As a user
  I want to use OpenAI or Anthropic as my LLM provider
  So that I can choose the best model for my needs

  Scenario: Select OpenAI provider via config
    Given a config with provider "openai" and api_key "sk-test"
    When I create a provider from config
    Then the provider should be "openai"

  Scenario: Select Anthropic provider via config
    Given a config with provider "anthropic" and api_key "sk-ant-test"
    When I create a provider from config
    Then the provider should be "anthropic"

  Scenario: Error classification distinguishes retryable errors
    Given a provider error with status 429
    Then the error should be classified as "rate_limit"
    And the error should be retryable

  Scenario: Error classification for auth errors
    Given a provider error with status 401
    Then the error should be classified as "auth"
    And the error should not be retryable

  Scenario: Error classification for server errors
    Given a provider error with status 500
    Then the error should be classified as "server"
    And the error should be retryable

  Scenario: Provider fallback on server error
    Given a primary provider that returns a server error "HTTP 500 Internal Server Error"
    And a fallback provider that returns "Fallback response"
    When I send a chat request through the fallback provider
    Then the fallback response content should be "Fallback response"

  Scenario: Provider respects cooldown after rate limit
    Given a primary provider that returns a rate limit error "HTTP 429 rate limit"
    And a fallback provider that returns "Cooldown fallback"
    When I send a chat request through the fallback provider
    And I send a second chat request through the fallback provider
    Then the fallback response content should be "Cooldown fallback"

  Scenario: Provider sends chat request with tools
    Given an OpenAI provider with a mock server
    And the mock server returns a chat response with content "Hello!"
    When I send a chat request with message "Hi" and a tool "bash"
    Then the chat response content should be "Hello!"
    And the chat request should have included an Authorization header

  Scenario: Reject insecure provider API base URL
    Given a config with provider "openai", api_key "sk-test", and api_base "http://attacker.invalid/v1"
    When I create a provider from config
    Then no provider should be created

  Scenario: OpenAI provider handles streaming responses
    Given an OpenAI provider with a mock server
    And the mock server returns an OpenAI streaming response with content "Hello world"
    When I send a streaming chat request with message "Hi"
    Then the streaming response content should be "Hello world"

  Scenario: Anthropic provider handles streaming responses
    Given an Anthropic provider with a mock server
    And the mock server returns an Anthropic streaming response with content "Hello from Claude"
    When I send a streaming chat request with message "Hi"
    Then the streaming response content should be "Hello from Claude"

  Scenario: Claude model is routed to Anthropic provider, not OpenAI
    Given a fallback provider with OpenAI first and Anthropic second
    When I send a chat request with model "claude-opus-4-5"
    Then the request should be handled by the "anthropic" provider

  Scenario: GPT model is routed to OpenAI provider
    Given a fallback provider with OpenAI first and Anthropic second
    When I send a chat request with model "gpt-4o"
    Then the request should be handled by the "openai" provider

  Scenario: Claude model bypasses a failed OpenAI provider
    Given a fallback provider with a failing OpenAI and a succeeding Anthropic
    When I send a chat request with model "claude-sonnet-4-20250514"
    Then the request should succeed with the Anthropic response

  Scenario: Unknown model falls back through providers in order
    Given a fallback provider with OpenAI first and Anthropic second
    When I send a chat request with model "some-unknown-model"
    Then the request should be handled by the "openai" provider

  # --- #178: is_error flag on tool result messages ---

  Scenario: Anthropic provider sends is_error flag on tool result messages
    Given an Anthropic request with a tool result marked as error
    When I build the Anthropic tool result message
    Then the tool result JSON should contain "is_error" set to true

  Scenario: Anthropic provider sends is_error false for successful tool results
    Given an Anthropic request with a successful tool result
    When I build the Anthropic tool result message
    Then the tool result JSON should contain "is_error" set to false

  # --- #179: Beta headers for API key auth ---

  Scenario: Anthropic provider sends beta headers for API key auth
    Given an Anthropic provider with API key auth and a mock server
    When I send an Anthropic chat request
    Then the request should include the "anthropic-beta" header with "fine-grained-tool-streaming-2025-05-14"

  # --- #177: Stop reason extraction ---

  Scenario: Anthropic provider extracts stop_reason from non-streaming response
    Given an Anthropic mock server that returns stop_reason "end_turn"
    When I send an Anthropic chat request
    Then the response stop_reason should be "EndTurn"

  Scenario: Anthropic provider extracts stop_reason max_tokens
    Given an Anthropic mock server that returns stop_reason "max_tokens"
    When I send an Anthropic chat request
    Then the response stop_reason should be "MaxTokens"

  Scenario: Anthropic provider extracts stop_reason tool_use
    Given an Anthropic mock server that returns stop_reason "tool_use"
    When I send an Anthropic chat request
    Then the response stop_reason should be "ToolUse"

  Scenario: Anthropic provider extracts stop_reason from SSE message_delta
    Given an Anthropic mock server that streams with stop_reason "end_turn"
    When I send an Anthropic streaming chat request
    Then the response stop_reason should be "EndTurn"

  # --- #180: Usage fields from SSE stream ---

  Scenario: Anthropic provider extracts usage from SSE message_start and message_delta
    Given an Anthropic mock server that streams usage with input 100 and output 50
    When I send an Anthropic streaming chat request
    Then the response usage should have prompt_tokens 100 and completion_tokens 50

  Scenario: Anthropic provider extracts cache usage from SSE stream
    Given an Anthropic mock server that streams cache usage with read 80 and write 20
    When I send an Anthropic streaming chat request
    Then the response usage should have cache_read_tokens 80 and cache_write_tokens 20

  # --- #176: Prompt caching (cache_control markers) ---

  Scenario: Anthropic provider adds cache_control to system prompt
    Given an Anthropic request with a system prompt "You are helpful"
    When I build the Anthropic request body
    Then the system prompt should be a content block array with cache_control

  Scenario: Anthropic provider adds cache_control to last user message
    Given an Anthropic request with multiple user messages
    When I build the Anthropic request body
    Then the last user message content block should have cache_control

  # --- #187: Batch consecutive tool results ---

  Scenario: Anthropic provider batches consecutive tool results into single user message
    Given an Anthropic request with 3 consecutive tool result messages
    When I build the Anthropic messages
    Then the tool results should be batched into a single user message with 3 tool_result blocks

  Scenario: Anthropic provider keeps single tool result as-is
    Given an Anthropic request with 1 consecutive tool result messages
    When I build the Anthropic messages
    Then the tool result should be in a single user message with 1 tool_result block

  # --- #183: tool_choice parameter ---

  Scenario: Anthropic provider sends tool_choice auto
    Given an Anthropic request with tool_choice "auto"
    When I build the Anthropic request body with tool_choice
    Then the request body should contain tool_choice type "auto"

  Scenario: Anthropic provider sends tool_choice any
    Given an Anthropic request with tool_choice "any"
    When I build the Anthropic request body with tool_choice
    Then the request body should contain tool_choice type "any"

  Scenario: Anthropic provider sends tool_choice specific tool
    Given an Anthropic request with tool_choice for tool "bash"
    When I build the Anthropic request body with tool_choice
    Then the request body should contain tool_choice type "tool" with name "bash"

  # --- #186: metadata.user_id support ---

  Scenario: Anthropic provider sends metadata with user_id
    Given an Anthropic request with user_id "telegram_12345"
    When I build the Anthropic request body with metadata
    Then the request body should contain metadata with user_id "telegram_12345"

  Scenario: Anthropic provider omits metadata when not provided
    Given an Anthropic request without metadata
    When I build the Anthropic request body with metadata
    Then the request body should not contain a metadata field

  # --- #175: Extended thinking support ---

  Scenario: Anthropic provider sends adaptive thinking for supported models
    Given an Anthropic request with model "claude-sonnet-4-20250514" and thinking level "medium"
    When I build the Anthropic request body with thinking
    Then the request body should contain thinking type "enabled" with budget_tokens 10000
    And the request body should not contain a temperature field

  Scenario: Anthropic provider sends budget-based thinking for older models
    Given an Anthropic request with model "claude-3-5-sonnet-20241022" and thinking level "high"
    When I build the Anthropic request body with thinking
    Then the request body should contain thinking type "enabled" with budget_tokens 16384

  Scenario: Anthropic provider skips thinking when level is none
    Given an Anthropic request with model "claude-sonnet-4-20250514" and no thinking level
    When I build the Anthropic request body with thinking
    Then the request body should not contain a thinking field
    And the request body should contain a temperature field

  Scenario: Anthropic provider handles thinking content blocks in SSE
    Given an Anthropic SSE response with thinking content blocks
    When I parse the SSE response
    Then the response should contain text content only (thinking blocks excluded from content)

  Scenario: Anthropic provider sets max_tokens to at least budget_tokens
    Given an Anthropic request with model "claude-3-5-sonnet-20241022" and thinking level "high" and max_tokens 4096
    When I build the Anthropic request body with thinking
    Then the request body max_tokens should be at least 16384

  # --- #185: Per-call cost tracking ---

  Scenario: Cost is calculated for known Anthropic models
    Given usage data with 1000 prompt tokens and 500 completion tokens for model "claude-sonnet-4-6"
    When I calculate the cost
    Then the total cost should be approximately 0.0105 USD
    And the input cost should be approximately 0.003 USD
    And the output cost should be approximately 0.0075 USD

  Scenario: Cost is not calculated for unknown models
    Given usage data with 1000 prompt tokens and 500 completion tokens for model "unknown-model"
    When I calculate the cost
    Then cost should be None
