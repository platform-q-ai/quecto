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

  # --- #181: True incremental SSE streaming ---

  Scenario: StreamEvent TextDelta carries individual text tokens
    Given an Anthropic SSE chunk with a text_delta event containing "Hello"
    When I parse the SSE chunk as a stream event
    Then the stream event should be a TextDelta with text "Hello"

  Scenario: StreamEvent ThinkingDelta carries thinking tokens
    Given an Anthropic SSE chunk with a thinking_delta event containing "Let me think"
    When I parse the SSE chunk as a stream event
    Then the stream event should be a ThinkingDelta with text "Let me think"

  Scenario: StreamEvent ToolCallStart carries tool id and name
    Given an Anthropic SSE chunk with a content_block_start for tool "bash" with id "toolu_001"
    When I parse the SSE chunk as a stream event
    Then the stream event should be a ToolCallStart with id "toolu_001" and name "bash"

  Scenario: StreamEvent ToolCallDelta carries partial JSON argument
    Given an Anthropic SSE chunk with an input_json_delta containing "{\"cmd\":"
    When I parse the SSE chunk as a stream event
    Then the stream event should be a ToolCallDelta with partial "{\"cmd\":"

  Scenario: StreamEvent ToolCallEnd is emitted on content_block_stop for a tool
    Given an Anthropic SSE chunk with a content_block_stop for tool "bash" id "toolu_001" and accumulated input "{\"cmd\":\"ls\"}"
    When I parse the SSE chunk as a stream event
    Then the stream event should be a ToolCallEnd with id "toolu_001" name "bash" and arguments "{\"cmd\":\"ls\"}"

  Scenario: Incremental SSE stream emits TextDelta events before Done
    Given an Anthropic mock server that streams text "Hello world" in 3 chunks
    When I send an incremental streaming chat request
    Then I should receive TextDelta events totalling "Hello world"
    And the final event should be Done with content "Hello world"

  Scenario: Incremental SSE stream emits ToolCallStart then ToolCallDelta then ToolCallEnd then Done
    Given an Anthropic mock server that streams a tool call for "bash" with arguments "{\"command\":\"ls\"}"
    When I send an incremental streaming chat request
    Then I should receive a ToolCallStart event for tool "bash"
    And I should receive ToolCallDelta events
    And I should receive a ToolCallEnd event for tool "bash" with arguments "{\"command\":\"ls\"}"
    And the final event should be Done with a tool call for "bash"

  Scenario: Incremental SSE stream emits Error event on HTTP failure
    Given an Anthropic mock server that returns an HTTP 500 error
    When I send an incremental streaming chat request
    Then I should receive an Error stream event

  Scenario: Incremental SSE stream handles chunked byte boundaries gracefully
    Given an Anthropic mock server that sends SSE lines split across byte chunks
    When I send an incremental streaming chat request
    Then I should receive TextDelta events totalling the expected text
    And no parse errors should occur

  Scenario: chat_stream_incremental assembles same LlmResponse as chat_stream
    Given an Anthropic mock server that streams a complete response with text and tool call
    When I send both a streaming and an incremental streaming chat request
    Then both responses should have identical content and tool calls

  # --- #184: Cross-provider message normalization pipeline ---

  # Tool call ID normalization
  Scenario: Tool call IDs with invalid characters are normalized before sending
    Given a message history with an assistant tool call id "call|with|pipes" for tool "bash"
    And a matching tool result for id "call|with|pipes"
    When I build Anthropic messages from that history
    Then the tool_use block should have id "call_with_pipes"
    And the tool_result block should have tool_use_id "call_with_pipes"

  Scenario: Tool call IDs longer than 64 characters are truncated
    Given a message history with an assistant tool call id "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaXXX" for tool "bash"
    And a matching tool result for id "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaXXX"
    When I build Anthropic messages from that history
    Then the tool_use block should have id "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    And the tool_result block should have tool_use_id "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"

  Scenario: Tool call IDs with valid characters pass through unchanged
    Given a message history with an assistant tool call id "valid-ID_123" for tool "bash"
    And a matching tool result for id "valid-ID_123"
    When I build Anthropic messages from that history
    Then the tool_use block should have id "valid-ID_123"
    And the tool_result block should have tool_use_id "valid-ID_123"

  Scenario: Codex-style long pipe-delimited tool call IDs are normalized
    Given a message history with an assistant tool call id "call_abc123|call_abc123|0" for tool "grep"
    And a matching tool result for id "call_abc123|call_abc123|0"
    When I build Anthropic messages from that history
    Then the tool_use block should have id "call_abc123_call_abc123_0"
    And the tool_result block should have tool_use_id "call_abc123_call_abc123_0"

  # Orphaned tool call detection
  Scenario: Orphaned tool call without a matching result gets a synthetic error result
    Given a message history with an assistant tool call id "orphan-id-1" for tool "bash" and no tool result
    When I build Anthropic messages from that history
    Then a synthetic tool result with tool_use_id "orphan-id-1" is injected
    And the synthetic result has content "No result provided" and is_error true

  Scenario: Multiple orphaned tool calls each get a synthetic error result
    Given a message history with two orphaned assistant tool calls "orphan-a" and "orphan-b"
    When I build Anthropic messages from that history
    Then a synthetic tool result with tool_use_id "orphan-a" is injected
    And a synthetic tool result with tool_use_id "orphan-b" is injected

  Scenario: Tool call with a matching result is not treated as orphaned
    Given a message history with an assistant tool call id "matched-id" for tool "bash"
    And a matching tool result for id "matched-id"
    When I build Anthropic messages from that history
    Then no synthetic tool result is injected for id "matched-id"

  # Message filtering
  Scenario: Assistant message with stop_reason error is filtered out before sending
    Given a message history containing an assistant message with stop_reason "error"
    When I build Anthropic messages from that history
    Then the errored assistant message is not present in the API payload

  Scenario: Assistant message with no stop_reason is not filtered out
    Given a message history containing an assistant message with stop_reason ""
    When I build Anthropic messages from that history
    Then the assistant message is present in the API payload

  # --- #188: User message content block support (inline images + capability filtering) ---

  # Plain text user messages — backward compat (no image blocks)
  Scenario: Plain text user message is sent as a simple string
    Given a user message with text "hello world" and no image blocks
    When I build Anthropic messages from that history for model "claude-opus-4-5"
    Then the user message content should be the string "hello world"

  # User message with image blocks — structured content array
  Scenario: User message with one image block is sent as a content block array
    Given a user message with text "look at this" and one image block of type "image/png"
    When I build Anthropic messages from that history for model "claude-opus-4-5"
    Then the user message content should be a block array
    And the block array should contain a text block "look at this"
    And the block array should contain an image block of media_type "image/png"

  Scenario: User message with multiple image blocks emits one text block and multiple image blocks
    Given a user message with text "compare these" and two image blocks of type "image/jpeg"
    When I build Anthropic messages from that history for model "claude-opus-4-5"
    Then the user message content should be a block array
    And the block array should contain a text block "compare these"
    And the block array should contain 2 image blocks

  # Vision capability filtering
  Scenario: Image blocks are filtered out for non-vision models
    Given a user message with text "look at this" and one image block of type "image/png"
    When I build Anthropic messages from that history for model "claude-instant-1"
    Then the user message content should be the string "look at this"

  Scenario: Image blocks are kept for vision-capable models
    Given a user message with text "look at this" and one image block of type "image/png"
    When I build Anthropic messages from that history for model "claude-3-opus-20240229"
    Then the user message content should be a block array
    And the block array should contain an image block of media_type "image/png"

  # Empty content filtering
  Scenario: User message with only whitespace text and no images is skipped
    Given a user message with text "   " and no image blocks
    When I build Anthropic messages from that history for model "claude-opus-4-5"
    Then the Anthropic payload should contain no user messages

  Scenario: User message with image but empty text emits only the image block
    Given a user message with text "" and one image block of type "image/webp"
    When I build Anthropic messages from that history for model "claude-opus-4-5"
    Then the user message content should be a block array
    And the block array should contain 1 image blocks
    And the block array should contain no text blocks

  Scenario: User message filtered to empty after removing images for non-vision model is skipped
    Given a user message with text "" and one image block of type "image/png"
    When I build Anthropic messages from that history for model "claude-instant-1"
    Then the Anthropic payload should contain no user messages
