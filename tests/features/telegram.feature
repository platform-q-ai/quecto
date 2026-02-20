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

  @pending
  Scenario: Handle Telegram bot commands
    Given a running gateway with Telegram enabled
    When user sends command "/status"
    Then the bot should respond with status information

  @pending
  Scenario: Graceful shutdown stops Telegram polling
    Given a running gateway with Telegram enabled
    When I send SIGINT to the gateway
    Then the Telegram channel should stop cleanly
