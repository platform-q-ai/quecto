Feature: E2E Real LLM Gateway
  Real endpoint tests for the gateway event loop via a mock Telegram API.

  @done @real-llm @real-llm-smoke
  Scenario: Gateway routes Telegram message to real LLM and sends reply
    Given a real LLM gateway workspace is configured for chat "12345" with message "Reply with exactly GATEWAY_OK"
    When I run quecto gateway until at least 1 Telegram replies are sent
    Then the Telegram outbound messages should include "GATEWAY_OK"

  @done @real-llm
  Scenario: Gateway supports multi-turn memory for same chat
    Given a real LLM gateway workspace is configured for chat "12345" with two messages "Remember this token: olive-314. Reply ACK_OLIVE" and "What token did I ask you to remember? Reply with only the token."
    When I run quecto gateway until at least 2 Telegram replies are sent
    Then the Telegram outbound messages should include "olive-314"

  @done @real-llm @real-llm-smoke
  Scenario: Gateway drops unauthorized Telegram users
    Given a real LLM gateway workspace is configured with allow_from "11111" and an update from user "22222" with message "Reply with SHOULD_NOT_APPEAR"
    When I run quecto gateway until at least 0 Telegram replies are sent
    Then the Telegram outbound messages should be empty
