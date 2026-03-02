@done
Feature: Telegram Gateway
  As a user
  I want to interact with Quecto through Telegram
  So that I can chat with my AI assistant from my phone

  Scenario: Telegram channel is created from config
    Given a config with Telegram enabled and token "123456:ABC" and allow_from "12345"
    When the Telegram channel is created
    Then the channel name should be "telegram"
    And the channel should be enabled

  Scenario: Telegram channel disabled when no token
    Given a config with Telegram disabled
    When I check if Telegram is enabled
    Then the Telegram channel should not be enabled

  Scenario: Allowed user passes the filter
    Given a Telegram channel with allow_from "12345", "67890"
    When user "12345" sends a message
    Then the message should pass the allow_from filter

  Scenario: Unauthorized user is rejected
    Given a Telegram channel with allow_from "12345", "67890"
    When user "99999" sends a message
    Then the message should be rejected by the allow_from filter

  Scenario: Empty allow_from rejects all users (fail closed)
    Given a Telegram channel with empty allow_from
    When user "99999" sends a message
    Then the message should be rejected by the allow_from filter

  Scenario: Incoming message is parsed correctly
    Given a raw Telegram update with text "Hello agent" from user "12345"
    When the update is parsed
    Then the parsed message text should be "Hello agent"
    And the parsed sender ID should be "12345"

  @done
  Scenario: Bot responds to /start command
    Given a running gateway with Telegram enabled and a mock Telegram API
    When user "12345" sends command "/start"
    Then the bot should respond with a welcome message to chat "12345"
    And the response should contain "quecto"

  @done
  Scenario: Bot responds to /help command
    Given a running gateway with Telegram enabled and a mock Telegram API
    When user "12345" sends command "/help"
    Then the bot should respond with available commands to chat "12345"
    And the response should contain "/start"
    And the response should contain "/help"
    And the response should contain "/status"

  @done
  Scenario: Bot responds to /status command
    Given a running gateway with Telegram enabled and a mock Telegram API
    And a valid config with OpenAI API key set
    When user "12345" sends command "/status"
    Then the bot should respond with status information to chat "12345"
    And the response should contain "Model:"

  @done
  Scenario: Unknown bot command is treated as regular message
    Given a running gateway with Telegram enabled and a mock LLM provider
    When user "12345" sends command "/unknown"
    Then the message should be routed to the agent as regular text

  @done
  Scenario: Graceful shutdown stops Telegram polling
    Given a running gateway with Telegram enabled and a mock Telegram API
    When the gateway receives a shutdown signal
    Then the Telegram polling loop should exit cleanly

  # --- /reload command ---

  @done
  Scenario: /help mentions /reload command
    Given a running gateway with Telegram enabled and a mock Telegram API
    When user "12345" sends command "/help"
    Then the response should contain "/reload"

  @done
  Scenario: /reload is recognised as a bot command and not routed to the agent
    Given a running gateway with Telegram enabled and a mock Telegram API
    When user "12345" sends command "/reload"
    Then the bot should respond with a reload confirmation to chat "12345"

  @done
  Scenario: strip_tool_history keeps user and plain assistant messages
    Given a session with messages:
      | role      | content        | is_manifest | tool_name |
      | User      | hello          | false       |           |
      | Assistant | sure thing     | false       |           |
    When strip_tool_history is applied
    Then the filtered messages should have 2 messages
    And message 0 should have role "User" and content "hello"
    And message 1 should have role "Assistant" and content "sure thing"

  @done
  Scenario: strip_tool_history drops manifest messages
    Given a session with messages:
      | role      | content           | is_manifest | tool_name |
      | User      | hello             | false       |           |
      | Assistant | [spill manifest]  | true        |           |
    When strip_tool_history is applied
    Then the filtered messages should have 1 messages
    And message 0 should have role "User" and content "hello"

  @done
  Scenario: strip_tool_history drops stale tool results (non-recall)
    Given a session with messages:
      | role      | content         | is_manifest | tool_name |
      | User      | do something    | false       |           |
      | Assistant |                 | false       | bash      |
      | Tool      | bash output     | false       | bash      |
    When strip_tool_history is applied
    Then the filtered messages should have 1 messages
    And message 0 should have role "User" and content "do something"

  @done
  Scenario: strip_tool_history keeps recall tool results and their paired assistant message
    Given a session with messages:
      | role      | content             | is_manifest | tool_name |
      | User      | what did we do?     | false       |           |
      | Assistant | (calls recall)      | false       | recall    |
      | Tool      | recalled content    | false       | recall    |
    When strip_tool_history is applied
    Then the filtered messages should have 3 messages
    And message 0 should have role "User" and content "what did we do?"
    And message 1 should have role "Assistant" and content "(calls recall)"
    And message 2 should have role "Tool" and content "recalled content"

  @done
  Scenario: strip_tool_history preserves narrative text from mixed assistant messages
    Given a session with messages:
      | role      | content            | is_manifest | tool_name |
      | User      | do something       | false       |           |
      | Assistant | I will run bash    | false       | bash      |
      | Tool      | bash output        | false       | bash      |
    When strip_tool_history is applied
    Then the filtered messages should have 2 messages
    And message 0 should have role "User" and content "do something"
    And message 1 should have role "Assistant" and content "I will run bash"

  @done
  Scenario: strip_tool_history drops pure tool-dispatch assistant messages with no text
    Given a session with messages:
      | role      | content       | is_manifest | tool_name |
      | User      | run it        | false       |           |
      | Assistant |               | false       | bash      |
      | Tool      | tool result   | false       | bash      |
    When strip_tool_history is applied
    Then the filtered messages should have 1 messages
    And message 0 should have role "User" and content "run it"

  @done
  Scenario: /reload on a session with stale tool history strips it and saves
    Given a session "telegram:99999" with stale tool calls exists in the store
    When the reload command is executed for chat "99999"
    Then the saved session "telegram:99999" should have no stale tool results
    And the reload response should contain "reloaded"
    And the reload response should mention messages kept and removed

  @done
  Scenario: /reload clears spill.jsonl for the session
    Given a session "telegram:99999" with stale tool calls exists in the store
    And the session has spill entries in the spill store
    When the reload command is executed for chat "99999"
    Then the spill file for session "telegram:99999" should be empty

  # --- Gateway session key correctness ---

  Scenario: Gateway session key uses chat_id without double-prefix
    Given an inbound message from source "telegram:12345"
    When the inbound processor loads the session
    Then the session key should be "telegram:12345"
    And the session key should not contain "telegram:telegram:"

  Scenario: /reload finds the session saved by the inbound processor
    Given a gateway inbound processor has handled one message from chat "55555"
    When the reload command is executed for chat "55555"
    Then the reload response should contain "reloaded"
    And the reload response should not contain "No existing session found"

  Scenario: Multi-turn gateway session persists history across two messages
    Given a gateway inbound processor has handled one message from chat "77777"
    When the gateway inbound processor handles a second message from chat "77777"
    Then the session for "telegram:77777" should contain 4 messages
