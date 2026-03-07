Feature: E2E Real LLM UDS Agent
  End-to-end tests for the UDS agent mode using real LLM providers.
  These exercise actual Anthropic and OpenAI OAuth credential paths,
  set_model routing, error handling, and recovery.

  Background:
    Given a real LLM UDS workspace is configured

  # ─── Happy path: Anthropic qualified model ────────────────────────────────

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
  Scenario: UDS switch models and prompt each successfully
    When I start the real LLM UDS agent
    And I send set_model "anthropic/claude-sonnet-4-20250514"
    And I send prompt "Reply with exactly UDS_SWITCH_A"
    And I send set_model "anthropic/claude-opus-4-6"
    And I send prompt "Reply with exactly UDS_SWITCH_B"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output event "agent_end" should appear 2 times

  # ─── Happy path: get_state reflects set_model ─────────────────────────────

  @done @real-llm
  Scenario: UDS get_state reflects model change
    When I start the real LLM UDS agent
    And I send set_model "anthropic/claude-sonnet-4-20250514"
    And I send command "get_state" with id "gs-1"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the get_state response model should be "anthropic/claude-sonnet-4-20250514"

  # ─── Sad path: nonexistent provider prefix ─────────────────────────────────

  @done @real-llm
  Scenario: UDS prompt with nonexistent provider returns error without crashing
    When I start the real LLM UDS agent
    And I send set_model "gemini/gemini-pro"
    And I send prompt "hello"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output should contain an agent_error event
    And the agent_error event should mention "no configured provider"

  @done @real-llm
  Scenario: UDS agent recovers after nonexistent provider error
    When I start the real LLM UDS agent
    And I send set_model "gemini/gemini-pro"
    And I send prompt "first"
    And I send set_model "anthropic/claude-sonnet-4-20250514"
    And I send prompt "Reply with exactly UDS_RECOVER_OK"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output should contain an event of type "agent_end"
    And the agent_end messages should contain "UDS_RECOVER_OK"

  # ─── Sad path: bare model name routing ─────────────────────────────────────

  @done @real-llm
  Scenario: UDS prompt with bare model name errors then recovers with qualified name
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

  # ─── Streaming: token events emitted ───────────────────────────────────────

  @done @real-llm
  Scenario: UDS prompt with real LLM emits token events
    When I start the real LLM UDS agent
    And I send prompt "Reply with exactly UDS_TOKENS_OK"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output should contain an event of type "token"
    And the agent output should contain an event of type "agent_end"

  # ─── Multi-prompt session continuity ───────────────────────────────────────

  @done @real-llm
  Scenario: UDS multi-prompt preserves context
    When I start the real LLM UDS agent
    And I send prompt "Remember the code word: pineapple-72. Reply ACK_REMEMBER"
    And I send prompt "What was the code word I told you? Reply with just the code word."
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output event "agent_end" should appear 2 times
    And the agent_end messages should contain "pineapple-72"

  # ─── Empty/invalid set_model args ──────────────────────────────────────────

  @done @real-llm
  Scenario: UDS set_model with empty string returns error
    When I start the real LLM UDS agent
    And I send set_model ""
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output should contain a response command "set_model" with success false
