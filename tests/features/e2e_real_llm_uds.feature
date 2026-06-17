Feature: E2E Real LLM UDS Agent
  End-to-end tests for the UDS agent mode using real LLM providers.
  These exercise actual Anthropic and OpenAI OAuth credential paths,
  set_model routing, error handling, and recovery.

  Covers ALL UDS commands:
    prompt, steer, follow_up, abort, get_state, get_messages,
    get_messages_tail, get_session_stats, set_model, get_extensions,
    reload_extensions

  Background:
    Given a real LLM UDS workspace is configured

  # ═══════════════════════════════════════════════════════════════════════════
  # prompt command
  # ═══════════════════════════════════════════════════════════════════════════

  @done @real-llm
  Scenario: UDS prompt with default Anthropic model succeeds
    When I start the real LLM UDS agent
    And I send prompt "Reply with exactly UDS_ANTHRO_OK"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output should contain an event of type "agent_start"
    And the agent output should contain an event of type "turn_end"
    And the agent output should contain an event of type "agent_end"
    And the agent_end messages should contain "UDS_ANTHRO_OK"

  @done @real-llm
  Scenario: UDS prompt emits token events during streaming
    When I start the real LLM UDS agent
    And I send prompt "Reply with exactly UDS_TOKENS_OK"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output should contain an event of type "token"
    And the agent output should contain an event of type "agent_end"

  @done @real-llm
  Scenario: UDS prompt with correlation id echoes id in response
    When I start the real LLM UDS agent
    And I send prompt with id "req-llm-42" and [message] "Reply with exactly UDS_ID_OK"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output should contain a response with id "req-llm-42"
    And the agent output should contain an event of type "agent_end"

  @done @real-llm
  Scenario: UDS multiple sequential prompts are processed
    When I start the real LLM UDS agent
    And I send prompt "Reply with exactly UDS_MULTI_A"
    And I send prompt "Reply with exactly UDS_MULTI_B"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output event "agent_end" should appear 2 times

  @done @real-llm
  Scenario: UDS multi-prompt preserves context
    When I start the real LLM UDS agent
    And I send prompt "Remember the code word: pineapple-72. Reply ACK_REMEMBER"
    And I send prompt "What was the code word I told you? Reply with just the code word."
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output event "agent_end" should appear 2 times
    And the agent_end messages should contain "pineapple-72"

  # ═══════════════════════════════════════════════════════════════════════════
  # set_model command
  # ═══════════════════════════════════════════════════════════════════════════

  @done @real-llm
  Scenario: UDS set_model to qualified Anthropic model and prompt
    When I start the real LLM UDS agent
    And I send set_model "anthropic/claude-sonnet-4-20250514"
    And I send prompt "Reply with exactly UDS_SONNET_OK"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output should contain a response command "set_model" with success true
    And the agent output should contain an event of type "agent_end"
    And the agent_end messages should contain "UDS_SONNET_OK"

  @done @real-llm
  Scenario: UDS set_model with provider and modelId fields
    When I start the real LLM UDS agent
    And I send set_model provider "anthropic" modelId "claude-sonnet-4-20250514"
    And I send command "get_state" with id "gs-quecto"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output should contain a response command "set_model" with success true
    And the get_state response model should be "anthropic/claude-sonnet-4-20250514"

  @done @real-llm
  Scenario: UDS switch models and prompt each successfully
    When I start the real LLM UDS agent
    And I send set_model "anthropic/claude-sonnet-4-20250514"
    And I send prompt "Reply with exactly UDS_SWITCH_A"
    And I send set_model "anthropic/claude-opus-4-6"
    And I send prompt "Reply with exactly UDS_SWITCH_B"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output event "agent_end" should appear 2 times

  @done @real-llm
  Scenario: UDS set_model with empty string returns error
    When I start the real LLM UDS agent
    And I send set_model ""
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output should contain a response command "set_model" with success false

  @done @real-llm
  Scenario: UDS set_model with nonexistent provider prefix
    When I start the real LLM UDS agent
    And I send set_model "gemini/gemini-pro"
    And I send prompt "hello"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output should contain an agent_error event
    And the agent_error event should mention "no configured provider"

  @done @real-llm
  Scenario: UDS agent recovers after set_model to nonexistent provider
    When I start the real LLM UDS agent
    And I send set_model "gemini/gemini-pro"
    And I send prompt "first"
    And I send set_model "anthropic/claude-sonnet-4-20250514"
    And I send prompt "Reply with exactly UDS_RECOVER_OK"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output should contain an event of type "agent_end"
    And the agent_end messages should contain "UDS_RECOVER_OK"

  @done @real-llm
  Scenario: UDS bare model name errors then recovers with qualified name
    When I start the real LLM UDS agent
    And I send set_model "claude-sonnet-4-20250514"
    And I send prompt "Reply with exactly UDS_BARE_FAIL"
    And I send set_model "anthropic/claude-sonnet-4-20250514"
    And I send prompt "Reply with exactly UDS_BARE_RECOVER"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output should contain an agent_error event
    And the agent output should contain an event of type "agent_end"
    And the agent_end messages should contain "UDS_BARE_RECOVER"

  # ═══════════════════════════════════════════════════════════════════════════
  # get_state command
  # ═══════════════════════════════════════════════════════════════════════════

  @done @real-llm
  Scenario: UDS get_state reflects model change
    When I start the real LLM UDS agent
    And I send set_model "anthropic/claude-sonnet-4-20250514"
    And I send command "get_state" with id "gs-1"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the get_state response model should be "anthropic/claude-sonnet-4-20250514"

  @done @real-llm
  Scenario: UDS get_state includes all expected fields
    When I start the real LLM UDS agent
    And I send prompt "Reply with exactly STATE_OK"
    And I send command "get_state" with id "gs-fields"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output should contain a response command "get_state" with success true
    And the get_state response should include field "isStreaming"
    And the get_state response should include field "messageCount"
    And the get_state response should include field "model"
    And the get_state response should include field "sessionKey"

  @done @real-llm
  Scenario: UDS get_state message count increases after prompts
    When I start the real LLM UDS agent
    And I send prompt "Reply with exactly COUNT_A"
    And I send prompt "Reply with exactly COUNT_B"
    And I send command "get_state" with id "gs-count"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the get_state response messageCount should be at least 4

  # ═══════════════════════════════════════════════════════════════════════════
  # get_messages command
  # ═══════════════════════════════════════════════════════════════════════════

  @done @real-llm
  Scenario: UDS get_messages returns conversation history after prompt
    When I start the real LLM UDS agent
    And I send prompt "Reply with exactly HISTORY_OK"
    And I send command "get_messages" with id "gm-1"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output should contain a response command "get_messages" with success true
    And the get_messages response data should include a "messages" array
    And the get_messages response should include a user [message] containing "HISTORY_OK"
    And the get_messages response should include an assistant [message]

  @done @real-llm
  Scenario: UDS get_messages on empty session returns empty array
    When I start the real LLM UDS agent
    And I send command "get_messages" with id "gm-empty"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output should contain a response command "get_messages" with success true

  # ═══════════════════════════════════════════════════════════════════════════
  # get_messages_tail command
  # ═══════════════════════════════════════════════════════════════════════════

  @done @real-llm
  Scenario: UDS get_messages_tail returns last N messages
    When I start the real LLM UDS agent
    And I send prompt "Reply with exactly TAIL_A"
    And I send prompt "Reply with exactly TAIL_B"
    And I send get_messages_tail with count 2 and id "gmt-1"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output should contain a response command "get_messages_tail" with success true
    And the get_messages_tail response should include a "messages" array
    And the get_messages_tail messages count should be exactly 2

  @done @real-llm
  Scenario: UDS get_messages_tail with count 0 returns empty array
    When I start the real LLM UDS agent
    And I send prompt "Reply with exactly TAIL_ZERO"
    And I send get_messages_tail with count 0 and id "gmt-zero"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output should contain a response command "get_messages_tail" with success true
    And the get_messages_tail messages count should be exactly 0

  @done @real-llm
  Scenario: UDS get_messages_tail with large count returns all messages
    When I start the real LLM UDS agent
    And I send prompt "Reply with exactly TAIL_ALL"
    And I send get_messages_tail with count 1000 and id "gmt-big"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output should contain a response command "get_messages_tail" with success true
    And the get_messages_tail response should include a "messages" array

  # ═══════════════════════════════════════════════════════════════════════════
  # get_session_stats command
  # ═══════════════════════════════════════════════════════════════════════════

  @done @real-llm
  Scenario: UDS get_session_stats returns token usage after prompt
    When I start the real LLM UDS agent
    And I send prompt "Reply with exactly STATS_OK"
    And I send command "get_session_stats" with id "ss-1"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output should contain a response command "get_session_stats" with success true
    And the get_session_stats response should include field "userMessages"
    And the get_session_stats response should include field "assistantMessages"
    And the get_session_stats response should include field "totalMessages"
    And the get_session_stats response should include field "tokens"
    And the get_session_stats userMessages should equal 1
    And the get_session_stats assistantMessages should equal 1

  # ═══════════════════════════════════════════════════════════════════════════
  # abort command
  # ═══════════════════════════════════════════════════════════════════════════

  @done @real-llm
  Scenario: UDS abort while idle returns success
    When I start the real LLM UDS agent
    And I send command "abort" with id "ab-idle"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output should contain a response command "abort" with success true

  # ═══════════════════════════════════════════════════════════════════════════
  # steer command
  # ═══════════════════════════════════════════════════════════════════════════

  @done @real-llm
  Scenario: UDS steer while idle is acknowledged
    When I start the real LLM UDS agent
    And I send steer "new direction" with id "st-idle"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output should contain a response command "steer" with success true

  # ═══════════════════════════════════════════════════════════════════════════
  # follow_up command
  # ═══════════════════════════════════════════════════════════════════════════

  @done @real-llm
  Scenario: UDS follow_up while idle is acknowledged
    When I start the real LLM UDS agent
    And I send follow_up "also do this" with id "fu-idle"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output should contain a response command "follow_up" with success true

  @done @real-llm
  Scenario: UDS follow_up message is processed after next prompt
    When I start the real LLM UDS agent
    And I send follow_up "Reply with exactly FOLLOWUP_PROCESSED" with id "fu-queue"
    And I send prompt "Reply with exactly PROMPT_FIRST"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output event "agent_end" should appear 2 times
    And the agent_end messages should contain "FOLLOWUP_PROCESSED"

  # ═══════════════════════════════════════════════════════════════════════════
  # get_extensions command
  # ═══════════════════════════════════════════════════════════════════════════

  @done @real-llm
  Scenario: UDS get_extensions returns list (empty when no extensions installed)
    When I start the real LLM UDS agent
    And I send command "get_extensions" with id "ge-1"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output should contain a response command "get_extensions" with success true

  # ═══════════════════════════════════════════════════════════════════════════
  # reload_extensions command
  # ═══════════════════════════════════════════════════════════════════════════

  @done @real-llm
  Scenario: UDS reload_extensions succeeds
    When I start the real LLM UDS agent
    And I send command "reload_extensions" with id "re-1"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output should contain a response command "reload_extensions" with success true

  # ═══════════════════════════════════════════════════════════════════════════
  # Correlation ID handling across commands
  # ═══════════════════════════════════════════════════════════════════════════

  @done @real-llm
  Scenario: UDS responses carry correlation ids for all command types
    When I start the real LLM UDS agent
    And I send command "get_state" with id "corr-gs"
    And I send command "get_messages" with id "corr-gm"
    And I send command "get_session_stats" with id "corr-ss"
    And I send command "get_extensions" with id "corr-ge"
    And I send command "abort" with id "corr-ab"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output should contain a response with id "corr-gs"
    And the agent output should contain a response with id "corr-gm"
    And the agent output should contain a response with id "corr-ss"
    And the agent output should contain a response with id "corr-ge"
    And the agent output should contain a response with id "corr-ab"
