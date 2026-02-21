@done
Feature: Telegram Gateway
  As a user
  I want to interact with Quecto through Telegram
  So that I can chat with my AI assistant from my phone

  Scenario: Telegram channel is created from config
    Given a config with Telegram enabled and token "123456:ABC"
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

  Scenario: Empty allow_from allows all users
    Given a Telegram channel with empty allow_from
    When user "99999" sends a message
    Then the message should pass the allow_from filter

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
