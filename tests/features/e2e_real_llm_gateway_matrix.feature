Feature: E2E Real LLM Gateway Matrix
  Broader real endpoint gateway coverage with mock Telegram transport.

  @done @real-llm
  Scenario: Gateway one-shot sentinel G1
    Given a real LLM gateway workspace is configured for chat "20001" with message "Reply with GATEWAY_G1"
    When I run quecto gateway until at least 1 Telegram replies are sent
    Then the Telegram outbound messages should include "GATEWAY_G1"

  @done @real-llm
  Scenario: Gateway one-shot sentinel G2
    Given a real LLM gateway workspace is configured for chat "20002" with message "Reply with GATEWAY_G2"
    When I run quecto gateway until at least 1 Telegram replies are sent
    Then the Telegram outbound messages should include "GATEWAY_G2"

  @done @real-llm
  Scenario: Gateway one-shot sentinel G3
    Given a real LLM gateway workspace is configured for chat "20003" with message "Reply with GATEWAY_G3"
    When I run quecto gateway until at least 1 Telegram replies are sent
    Then the Telegram outbound messages should include "GATEWAY_G3"

  @done @real-llm
  Scenario: Gateway handles bot help command directly
    Given a real LLM gateway workspace is configured for chat "20004" with message "/help"
    When I run quecto gateway until at least 1 Telegram replies are sent
    Then the Telegram outbound messages should include "Available commands"

  @done @real-llm
  Scenario: Gateway handles bot status command directly
    Given a real LLM gateway workspace is configured for chat "20005" with message "/status"
    When I run quecto gateway until at least 1 Telegram replies are sent
    Then the Telegram outbound messages should include "Model:"

  @done @real-llm
  Scenario: Gateway multi-turn memory M1
    Given a real LLM gateway workspace is configured for chat "20006" with two messages "Remember token gate-111" and "What token did I give you?"
    When I run quecto gateway until at least 2 Telegram replies are sent
    Then the Telegram outbound messages should include "gate-111"

  @done @real-llm
  Scenario: Gateway multi-turn memory M2
    Given a real LLM gateway workspace is configured for chat "20007" with two messages "Remember token gate-222" and "What token did I give you?"
    When I run quecto gateway until at least 2 Telegram replies are sent
    Then the Telegram outbound messages should include "gate-222"

  @done @real-llm
  Scenario: Gateway uses tools to create file from message
    Given a real LLM gateway workspace is configured for chat "20008" with message "Create file gateway-tool-a.txt containing GATE_TOOL_A"
    When I run quecto gateway until at least 1 Telegram replies are sent
    Then the file "gateway-tool-a.txt" should exist in the e2e workspace

  @done @real-llm
  Scenario: Gateway uses tools to create nested file
    Given a real LLM gateway workspace is configured for chat "20009" with message "Create file gw/nested-b.txt containing GATE_TOOL_B"
    When I run quecto gateway until at least 1 Telegram replies are sent
    Then the file "gw/nested-b.txt" should exist in the e2e workspace

  @done @real-llm
  Scenario: Gateway unauthorized user dropped U1
    Given a real LLM gateway workspace is configured with allow_from "20010" and an update from user "30010" with message "Reply with NOPE_U1"
    When I run quecto gateway until at least 0 Telegram replies are sent
    Then the Telegram outbound messages should be empty

  @done @real-llm
  Scenario: Gateway unauthorized user dropped U2
    Given a real LLM gateway workspace is configured with allow_from "20011" and an update from user "30011" with message "Reply with NOPE_U2"
    When I run quecto gateway until at least 0 Telegram replies are sent
    Then the Telegram outbound messages should be empty

  @done @real-llm
  Scenario: Gateway unknown slash command routes to LLM
    Given a real LLM gateway workspace is configured for chat "20012" with message "/unknowncmd include GATE_UNKNOWN_OK"
    When I run quecto gateway until at least 1 Telegram replies are sent
    Then the Telegram outbound messages should not be empty

  @done @real-llm
  Scenario: Gateway text message with exec request
    Given a real LLM gateway workspace is configured for chat "20013" with message "Run echo GATE_EXEC_OK and include GATE_EXEC_OK"
    When I run quecto gateway until at least 1 Telegram replies are sent
    Then the Telegram outbound messages should include "GATE_EXEC_OK"

  @done @real-llm
  Scenario: Gateway text message with missing file fallback
    Given a real LLM gateway workspace is configured for chat "20014" with message "Read no-gateway-file.txt and if missing reply GATE_MISS_OK"
    When I run quecto gateway until at least 1 Telegram replies are sent
    Then the Telegram outbound messages should include "GATE_MISS_OK"

  @done @real-llm
  Scenario: Gateway text message file listing response
    Given a real LLM gateway workspace is configured for chat "20015" with message "Create files ga.txt and gb.txt then mention ga.txt and gb.txt"
    When I run quecto gateway until at least 1 Telegram replies are sent
    Then the Telegram outbound messages should include "ga.txt"
