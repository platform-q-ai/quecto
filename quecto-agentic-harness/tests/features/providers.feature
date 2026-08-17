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

  Scenario: HTTP 529 is classified as server error and is retryable
    Given a provider error with status 529
    Then the error should be classified as "server"
    And the error should be retryable

  Scenario: Anthropic overloaded_error body text is classified as server error
    Given a provider error with [message] "HTTP 529 from Anthropic: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}"
    Then the error should be classified as "server"
    And the error should be retryable

  # --- #935: clamp per-model max_tokens to the model's output cap ---

  Scenario: A model's request output cap is clamped to the model's output limit
    Given a model whose output cap is 65536 tokens
    And a configured max_tokens of 200000
    When the agent builds a request for that model
    Then the request output cap should be 65536

  # --- #935: an explicit client error code wrapped in a 5xx is non-retryable ---

  Scenario: A 500 declaring an invalid_request_error client code fails fast as a client error
    Given a 500 response whose body declares an "invalid_request_error" client code
    Then the error should be classified as "client"
    And the error should not be retryable

  Scenario: Provider router forwards error without fallback
    Given a provider router with a failing OpenAI and a succeeding Anthropic
    When I send a chat request with model "gpt-4o" through the router
    Then the request should fail with a provider error

  # --- #931: bounded retry-with-backoff for transient provider errors ---

  Scenario: A transient server error is recovered transparently by retrying
    Given a retrying provider that fails 2 times with "provider error (503): service unavailable" then succeeds
    And the retry decorator allows up to 4 attempts
    When I send a chat request through the retrying provider
    Then the request eventually succeeds despite the transient failures

  Scenario: A rate-limit error is recovered transparently by retrying
    Given a retrying provider that fails 1 time with "provider error (429): rate limit exceeded" then succeeds
    And the retry decorator allows up to 4 attempts
    When I send a chat request through the retrying provider
    Then the request eventually succeeds despite the transient failures

  Scenario: A persistently failing transient error fails once the budget is exhausted
    Given a retrying provider that always fails with "provider error (500): internal server error"
    And the retry decorator allows up to 3 attempts
    When I send a chat request through the retrying provider
    Then the request fails after retries are exhausted

  Scenario: A client 4xx error fails immediately without retrying
    Given a retrying provider that always fails with "provider error (400): invalid_request_error"
    And the retry decorator allows up to 4 attempts
    When I send a chat request through the retrying provider
    Then the request fails without being retried

  Scenario: A cancelled request fails immediately without retrying
    Given a retrying provider that always fails with "request cancelled"
    And the retry decorator allows up to 4 attempts
    When I send a chat request through the retrying provider
    Then the request fails without being retried

  Scenario: Provider sends chat request with tools
    Given an OpenAI provider with a mock server
    And the mock server returns a chat response with content "Hello!"
    When I send a chat request with [message] "Hi" and a tool "bash"
    Then the chat response content should be "Hello!"
    And the chat request should have included an Authorization header

  Scenario: Reject insecure provider API base URL
    Given a config with provider "openai", api_key "sk-test", and api_base "http://attacker.invalid/v1"
    When I create a provider from config
    Then no provider should be created

  Scenario: OpenAI provider handles streaming responses
    Given an OpenAI provider with a mock server
    And the mock server returns an OpenAI streaming response with content "Hello world"
    When I send a streaming chat request with [message] "Hi"
    Then the streaming response content should be "Hello world"

  Scenario: Anthropic provider handles streaming responses
    Given an Anthropic provider with a mock server
    And the mock server returns an Anthropic streaming response with content "Hello from Claude"
    When I send a streaming chat request with [message] "Hi"
    Then the streaming response content should be "Hello from Claude"

  Scenario: Explicit anthropic/ prefix routes to Anthropic provider
    Given a provider router with OpenAI first and Anthropic second
    When I send a chat request with model "anthropic/claude-opus-4-5"
    Then the request should be handled by the "anthropic" provider

  Scenario: Explicit openai/ prefix routes to OpenAI provider
    Given a provider router with OpenAI first and Anthropic second
    When I send a chat request with model "openai/gpt-4o"
    Then the request should be handled by the "openai" provider

  Scenario: Opaque multi-segment model IDs route by first provider slash only
    Given a provider router with OpenAI first and Fireworks second
    When I send a chat request with model "fireworks/accounts/fireworks/models/glm-5p2"
    Then the request should be handled by the "fireworks" provider
    And the provider should receive model "accounts/fireworks/models/glm-5p2"

  Scenario: Unknown provider prefix does not fall back for multi-segment model IDs
    Given a provider router with OpenAI first and Anthropic second
    When I send a chat request with model "fireworks/accounts/fireworks/models/glm-5p2"
    Then the request should fail with no configured provider "fireworks"
    And the request should not be handled by the "openai" provider

  Scenario: Bare model name goes to first provider in order
    Given a provider router with OpenAI first and Anthropic second
    When I send a chat request with model "claude-opus-4-5"
    Then the request should be handled by the "openai" provider

  Scenario: Unknown model goes to first provider in order
    Given a provider router with OpenAI first and Anthropic second
    When I send a chat request with model "some-unknown-model"
    Then the request should be handled by the "openai" provider

  Scenario: Provider router forwards request without cloning messages
    Given a provider router with a single provider
    When I send a chat request through the router and track the messages pointer
    Then the provider should receive the same messages pointer as the caller

  # --- #178: is_error flag on tool result messages ---

  Scenario: Anthropic provider sends is_error flag on tool result messages
    Given an Anthropic request with a tool result marked as error
    When I build the Anthropic tool result [message]
    Then the tool result JSON should contain "is_error" set to true

  Scenario: Anthropic provider sends is_error false for successful tool results
    Given an Anthropic request with a successful tool result
    When I build the Anthropic tool result [message]
    Then the tool result JSON should contain "is_error" set to false

  # --- #179: Beta headers for API key auth ---
  # NOTE: fine-grained-tool-streaming was previously removed as "GA", but is
  # still required (#437-3). Updated to verify it IS sent for parity.

  Scenario: Anthropic provider sends fine-grained-tool-streaming beta header for API parity
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

  Scenario: Anthropic provider reports cached input as context occupancy, not billable prompt tokens
    Given an Anthropic mock server that streams cache usage with read 80 and write 20
    When I send an Anthropic streaming chat request
    Then the response usage should have prompt_tokens 100 and completion_tokens 50
    And context_tokens should be 200 for the context usage counter

  # --- #176: Prompt caching (cache_control markers) ---

  Scenario: Anthropic provider adds cache_control to system prompt
    Given an Anthropic request with a system prompt "You are helpful"
    When I build the Anthropic request body
    Then the system prompt should be a content block array with cache_control

  Scenario: Anthropic provider adds cache_control to last user message
    Given an Anthropic request with multiple user messages
    When I build the Anthropic request body
    Then the last user [message] content block should have cache_control

  # --- #187: Batch consecutive tool results ---

  Scenario: Anthropic provider batches consecutive tool results into single user message
    Given an Anthropic request with 3 consecutive tool result messages
    When I build the Anthropic messages
    Then the tool results should be batched into a single user [message] with 3 tool_result blocks

  Scenario: Anthropic provider keeps single tool result as-is
    Given an Anthropic request with 1 consecutive tool result messages
    When I build the Anthropic messages
    Then the tool result should be in a single user [message] with 1 tool_result block

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
    Given an Anthropic request with model "claude-sonnet-4-6" and thinking level "medium"
    When I build the Anthropic request body with thinking
    Then the request body should contain thinking type "adaptive"
    And the request body should not contain a temperature field

  Scenario: Anthropic provider sends budget-based thinking for older models
    Given an Anthropic request with model "claude-3-5-sonnet-20241022" and thinking level "high"
    When I build the Anthropic request body with thinking
    Then the request body should contain thinking type "enabled" with budget_tokens 16384

  Scenario: Anthropic provider skips thinking when level is none for older models
    Given an Anthropic request with model "claude-3-5-sonnet-20241022" and no thinking level
    When I build the Anthropic request body with thinking
    Then the request body should not contain a thinking field
    And the request body should contain a temperature field

  # --- #432: Auto-enable adaptive thinking for 4.6 models ---

  Scenario: Anthropic provider auto-enables adaptive thinking for Opus 4.6 even with no thinking level
    Given an Anthropic request with model "claude-opus-4-6" and no thinking level
    When I build the Anthropic request body with thinking
    Then the request body should contain thinking type "adaptive"
    And the request body should not contain a temperature field
    And the request body should contain output_config effort "low"

  Scenario: Anthropic provider omits deprecated temperature for Opus 4.7
    Given an Anthropic request with model "claude-opus-4-7" and no thinking level
    When I build the Anthropic request body with thinking
    Then the request body should contain thinking type "adaptive"
    And the request body should not contain a temperature field
    And the request body should contain output_config effort "low"

  Scenario: Anthropic provider omits deprecated temperature for Opus 4.8
    Given an Anthropic request with model "claude-opus-4-8" and no thinking level
    When I build the Anthropic request body with thinking
    Then the request body should contain thinking type "adaptive"
    And the request body should not contain a temperature field
    And the request body should contain output_config effort "low"

  Scenario: Anthropic provider auto-enables adaptive thinking for Sonnet 4.6 even with no thinking level
    Given an Anthropic request with model "claude-sonnet-4-6" and no thinking level
    When I build the Anthropic request body with thinking
    Then the request body should contain thinking type "adaptive"
    And the request body should not contain a temperature field
    And the request body should contain output_config effort "low"

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
    Given a [message] history with an assistant tool call id "call|with|pipes" for tool "bash"
    And a matching tool result for id "call|with|pipes"
    When I build Anthropic messages from that history
    Then the tool_use block should have id "call_with_pipes"
    And the tool_result block should have tool_use_id "call_with_pipes"

  Scenario: Tool call IDs longer than 64 characters are truncated
    Given a [message] history with an assistant tool call id "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaXXX" for tool "bash"
    And a matching tool result for id "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaXXX"
    When I build Anthropic messages from that history
    Then the tool_use block should have id "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    And the tool_result block should have tool_use_id "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"

  Scenario: Tool call IDs with valid characters pass through unchanged
    Given a [message] history with an assistant tool call id "valid-ID_123" for tool "bash"
    And a matching tool result for id "valid-ID_123"
    When I build Anthropic messages from that history
    Then the tool_use block should have id "valid-ID_123"
    And the tool_result block should have tool_use_id "valid-ID_123"

  Scenario: Codex-style long pipe-delimited tool call IDs are normalized
    Given a [message] history with an assistant tool call id "call_abc123|call_abc123|0" for tool "grep"
    And a matching tool result for id "call_abc123|call_abc123|0"
    When I build Anthropic messages from that history
    Then the tool_use block should have id "call_abc123_call_abc123_0"
    And the tool_result block should have tool_use_id "call_abc123_call_abc123_0"

  # Orphaned tool call detection
  Scenario: Orphaned tool call without a matching result gets a synthetic error result
    Given a [message] history with an assistant tool call id "orphan-id-1" for tool "bash" and no tool result
    When I build Anthropic messages from that history
    Then a synthetic tool result with tool_use_id "orphan-id-1" is injected
    And the synthetic result has content "No result provided" and is_error true

  Scenario: Multiple orphaned tool calls each get a synthetic error result
    Given a [message] history with two orphaned assistant tool calls "orphan-a" and "orphan-b"
    When I build Anthropic messages from that history
    Then a synthetic tool result with tool_use_id "orphan-a" is injected
    And a synthetic tool result with tool_use_id "orphan-b" is injected

  Scenario: Tool call with a matching result is not treated as orphaned
    Given a [message] history with an assistant tool call id "matched-id" for tool "bash"
    And a matching tool result for id "matched-id"
    When I build Anthropic messages from that history
    Then no synthetic tool result is injected for id "matched-id"

  # Message filtering
  Scenario: Assistant message with stop_reason error is filtered out before sending
    Given a [message] history containing an assistant message with stop_reason "error"
    When I build Anthropic messages from that history
    Then the errored assistant [message] is not present in the API payload

  Scenario: Assistant message with no stop_reason is not filtered out
    Given a [message] history containing an assistant message with stop_reason ""
    When I build Anthropic messages from that history
    Then the assistant [message] is present in the API payload

  # --- #188: User message content block support (inline images + capability filtering) ---

  # Plain text user messages — backward compat (no image blocks)
  Scenario: Plain text user message is sent as a simple string
    Given a user [message] with text "hello world" and no image blocks
    When I build Anthropic messages from that history for model "claude-opus-4-5"
    Then the user [message] content should be the string "hello world"

  # User message with image blocks — structured content array
  Scenario: User message with one image block is sent as a content block array
    Given a user [message] with text "look at this" and one image block of type "image/png"
    When I build Anthropic messages from that history for model "claude-opus-4-5"
    Then the user [message] content should be a block array
    And the block array should contain a text block "look at this"
    And the block array should contain an image block of media_type "image/png"

  Scenario: User message with multiple image blocks emits one text block and multiple image blocks
    Given a user [message] with text "compare these" and two image blocks of type "image/jpeg"
    When I build Anthropic messages from that history for model "claude-opus-4-5"
    Then the user [message] content should be a block array
    And the block array should contain a text block "compare these"
    And the block array should contain 2 image blocks

  # Vision capability filtering
  Scenario: Image blocks are filtered out for non-vision models
    Given a user [message] with text "look at this" and one image block of type "image/png"
    When I build Anthropic messages from that history for model "claude-instant-1"
    Then the user [message] content should be the string "look at this"

  Scenario: Image blocks are kept for vision-capable models
    Given a user [message] with text "look at this" and one image block of type "image/png"
    When I build Anthropic messages from that history for model "claude-3-opus-20240229"
    Then the user [message] content should be a block array
    And the block array should contain an image block of media_type "image/png"

  # --- #310: Vision allow-list (fail-closed for unknown models) ---

  Scenario: Unknown model is treated as non-vision (fail-closed)
    Given a user [message] with text "look at this" and one image block of type "image/png"
    When I build Anthropic messages from that history for model "unknown-future-model"
    Then the user [message] content should be the string "look at this"

  # Empty content filtering
  Scenario: User message with only whitespace text and no images is skipped
    Given a user [message] with text "   " and no image blocks
    When I build Anthropic messages from that history for model "claude-opus-4-5"
    Then the Anthropic payload should contain no user messages

  Scenario: User message with image but empty text emits only the image block
    Given a user [message] with text "" and one image block of type "image/webp"
    When I build Anthropic messages from that history for model "claude-opus-4-5"
    Then the user [message] content should be a block array
    And the block array should contain 1 image blocks
    And the block array should contain no text blocks

  Scenario: User message filtered to empty after removing images for non-vision model is skipped
    Given a user [message] with text "" and one image block of type "image/png"
    When I build Anthropic messages from that history for model "claude-instant-1"
    Then the Anthropic payload should contain no user messages

  # --- #182: Abort/cancellation support via CancelFlag ---

  Scenario: Chat request is cancelled before it starts
    Given an Anthropic mock server that returns a successful text response
    And a cancel flag that is already set
    When I send a chat request with the cancel flag
    Then the chat request should return a cancellation error

  Scenario: Streaming chat request is cancelled before it starts
    Given an Anthropic mock server that streams text "Hello world" in 3 chunks
    And a cancel flag that is already set
    When I send a streaming chat request with the cancel flag
    Then the streaming chat request should return a cancellation error

  Scenario: Chat request completes normally when cancel flag is not set
    Given an Anthropic mock server that returns a successful text response
    And a cancel flag that is not set
    When I send a chat request with the cancel flag
    Then the chat request should succeed with a response

  Scenario: Incremental streaming chat emits Error event when cancelled before start
    Given an Anthropic mock server that streams text "Hello world" in 3 chunks
    And a cancel flag that is already set
    When I send an incremental streaming chat request with the cancel flag
    Then I should receive an Error stream event containing "cancelled"

  Scenario: StopReason aborted is parsed correctly
    Given a stop reason string "aborted"
    When I parse the stop reason
    Then the stop reason should be Aborted

  Scenario: Aborted assistant messages are dropped from normalized message list
    Given a [message] list with an aborted assistant turn followed by a new user message
    When I normalize the messages
    Then the aborted assistant [message] should be removed
    And the new user [message] should remain

  # --- #416: Default effort=low for 4.6 models; model_context_window_exceeded ---

  Scenario: Sonnet 4.6 with no effort emits effort=low and adaptive thinking in request body
    Given an Anthropic request for model "claude-sonnet-4-6" with no effort level
    When I build the Anthropic request body with effort
    Then the request body should contain output_config effort "low"
    And the request body should contain thinking type "adaptive"
    And the request body should not contain a temperature field

  Scenario: Opus 4.6 with no effort emits effort=low and adaptive thinking in request body
    Given an Anthropic request for model "claude-opus-4-6" with no effort level
    When I build the Anthropic request body with effort
    Then the request body should contain output_config effort "low"
    And the request body should contain thinking type "adaptive"
    And the request body should not contain a temperature field

  Scenario: Sonnet 4.6 with explicit effort=medium uses the override with adaptive thinking
    Given an Anthropic request for model "claude-sonnet-4-6" with effort level "medium"
    When I build the Anthropic request body with effort
    Then the request body should contain output_config effort "medium"
    And the request body should contain thinking type "adaptive"

  Scenario: Non-4.6 model with no effort omits output_config
    Given an Anthropic request for model "claude-opus-4-5" with no effort level
    When I build the Anthropic request body with effort
    Then the request body should not contain an output_config field

  Scenario: StopReason model_context_window_exceeded is parsed as MaxTokens
    Given a stop reason string "model_context_window_exceeded"
    When I parse the stop reason
    Then the stop reason should be MaxTokens

  @done
  Scenario: normalize_messages does not clone messages that need no modification
    Given a [message] list with only user and assistant messages and no tool calls
    When I normalize the messages
    Then all messages should be returned without deep cloning

  # --- #437: Anthropic provider API parity ---

  # #437-1: System prompt for OAuth
  Scenario: OAuth token prepends identity system prompt
    Given an Anthropic request with system prompt "Be helpful" and is_oauth true
    When I build the Anthropic request body with OAuth
    Then the system prompt array should have 2 blocks
    And the first system block text should be "You are Claude Code, Anthropic's official CLI for Claude."
    And the second system block text should be "Be helpful"
    And both system blocks should have cache_control ephemeral

  Scenario: OAuth token without system prompt still includes identity prefix
    Given an Anthropic request with no system prompt and is_oauth true
    When I build the Anthropic request body with OAuth
    Then the system prompt array should have 1 block
    And the first system block text should be "You are Claude Code, Anthropic's official CLI for Claude."

  Scenario: API key auth does not prepend identity system prompt
    Given an Anthropic request with system prompt "Be helpful" and is_oauth false
    When I build the Anthropic request body without OAuth
    Then the system prompt array should have 1 block
    And the first system block text should be "Be helpful"

  # #437-2,3,7,9: Beta headers
  Scenario: API key auth for non-4.6 model sends interleaved-thinking and fine-grained-tool-streaming betas
    Given an Anthropic beta header for model "claude-sonnet-4-5" with is_oauth false
    When I build the beta header
    Then the beta header should contain "fine-grained-tool-streaming-2025-05-14"
    And the beta header should contain "interleaved-thinking-2025-05-14"
    And the beta header should not contain "claude-code-20250219"

  Scenario: API key auth for 4.6 model omits interleaved-thinking but keeps fine-grained-tool-streaming
    Given an Anthropic beta header for model "claude-opus-4-6" with is_oauth false
    When I build the beta header
    Then the beta header should contain "fine-grained-tool-streaming-2025-05-14"
    And the beta header should not contain "interleaved-thinking-2025-05-14"

  Scenario: OAuth auth includes identity and oauth betas plus streaming betas
    Given an Anthropic beta header for model "claude-sonnet-4-5" with is_oauth true
    When I build the beta header
    Then the beta header should contain "claude-code-20250219"
    And the beta header should contain "oauth-2025-04-20"
    And the beta header should contain "fine-grained-tool-streaming-2025-05-14"
    And the beta header should contain "interleaved-thinking-2025-05-14"

  # #437-4: Tool name remapping for OAuth
  Scenario: OAuth mode remaps tool names to canonical casing
    Given a tool named "read"
    When I convert it to canonical name
    Then the result should be "Read"

  Scenario: OAuth mode remaps "bash" to "Bash"
    Given a tool named "bash"
    When I convert it to canonical name
    Then the result should be "Bash"

  Scenario: Unknown tool names pass through unchanged
    Given a tool named "my_custom_tool"
    When I convert it to canonical name
    Then the result should be "my_custom_tool"

  Scenario: Tool definitions are remapped in OAuth mode
    Given an Anthropic request with tool "read" and is_oauth true
    When I build the Anthropic request body with OAuth
    Then the Anthropic tool definition name should be "Read"

  Scenario: Tool definitions are NOT remapped in API key mode
    Given an Anthropic request with tool "read" and is_oauth false
    When I build the Anthropic request body without OAuth
    Then the Anthropic tool definition name should be "read"

  # #437-5: Thinking block replay in multi-turn conversations
  Scenario: Assistant message with normal thinking block includes thinking in API payload
    Given an assistant [message] with a normal thinking block "Let me reason" and signature "sig123"
    When I build the Anthropic assistant [message]
    Then the content blocks should include a thinking block with text "Let me reason" and signature "sig123"

  Scenario: Assistant message with redacted thinking block includes redacted_thinking in API payload
    Given an assistant [message] with a redacted thinking block with data "opaque_data_abc"
    When I build the Anthropic assistant [message]
    Then the content blocks should include a redacted_thinking block with data "opaque_data_abc"

  Scenario: Thinking block with empty signature is not replayed as answer text
    Given an assistant [message] with a normal thinking block "some reasoning" and signature ""
    When I build the Anthropic assistant [message]
    Then the content blocks should not include provider thinking text "some reasoning"

  # #437-6: signature_delta SSE handling
  Scenario: SSE signature_delta events accumulate the thinking block signature
    Given an Anthropic SSE stream with thinking_delta "reasoning" and signature_delta "sig_abc"
    When I parse the SSE events
    Then the accumulated thinking block should have signature "sig_abc"

  # #437-10: Accept header
  Scenario: Anthropic requests include Accept application/json header
    Given an Anthropic provider with a mock server expecting Accept header
    When I send an Anthropic chat request
    Then the request should include header "Accept" with value "application/json"

  # #437-15: pause_turn stop reason
  Scenario: StopReason pause_turn is parsed as EndTurn
    Given a stop reason string "pause_turn"
    When I parse the stop reason
    Then the stop reason should be EndTurn

  # #437-16: sensitive stop reason
  Scenario: StopReason sensitive is parsed as Error
    Given a stop reason string "sensitive"
    When I parse the stop reason
    Then the stop reason should be Error

  # #438: SSE streaming reverse-maps OAuth tool names back to registry names
  Scenario: SSE batch parse reverse-maps PascalCase tool names in OAuth mode
    Given an Anthropic SSE response with tool "Read" and tool definitions for "read"
    When I parse the SSE response with OAuth tool remapping
    Then the tool call name in the response should be "read"

  Scenario: SSE batch parse passes through tool names when no remapping is configured
    Given an Anthropic SSE response with tool "read" and no tool remapping
    When I parse the SSE response without OAuth tool remapping
    Then the tool call name in the response should be "read"

  Scenario: SSE incremental stream reverse-maps PascalCase tool names in OAuth mode
    Given an Anthropic SSE response with tool "Bash" and tool definitions for "bash"
    When I parse the SSE events with OAuth tool remapping
    Then the ToolCallStart event name should be "bash"
    And the ToolCallEnd event name should be "bash"
    And the Done response tool call name should be "bash"

  Scenario: SSE incremental stream passes through names when no remapping is configured
    Given an Anthropic SSE response with tool "bash" and no tool remapping
    When I parse the SSE events without OAuth tool remapping
    Then the ToolCallStart event name should be "bash"
    And the ToolCallEnd event name should be "bash"
    And the Done response tool call name should be "bash"
