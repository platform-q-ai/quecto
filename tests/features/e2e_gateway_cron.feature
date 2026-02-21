@pending
Feature: End-to-End Gateway Cron Integration
  As a system operator
  I want the gateway to execute scheduled cron jobs automatically
  So that recurring tasks fire on time without manual intervention

  The gateway event loop should include a cron tick timer that checks for
  due jobs and dispatches them through the agent. These tests verify the
  gateway actually wires cron execution into its event loop, not just that
  execute_cron_tick() works when called directly.

  Background:
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server

  # --- Basic cron execution ---

  Scenario: Gateway executes a due cron job through the agent
    Given a cron job "weather-check" with interval 2 seconds and message "Check the weather"
    And the mock LLM returns a text response "Weather is sunny"
    When I run quecto gateway for at least 5 seconds
    Then the mock LLM should have received at least 1 request containing "Check the weather"

  Scenario: Gateway executes multiple due cron jobs
    Given a cron job "weather" with interval 2 seconds and message "Check weather"
    And a cron job "backup" with interval 2 seconds and message "Run backup"
    And the mock LLM returns a text response "Task done"
    When I run quecto gateway for at least 5 seconds
    Then the mock LLM should have received at least 1 request containing "Check weather"
    And the mock LLM should have received at least 1 request containing "Run backup"

  # --- Disabled jobs ---

  Scenario: Gateway skips disabled cron jobs
    Given a disabled cron job "weather" with interval 2 seconds and message "Check weather"
    And the mock LLM returns a text response "Should not happen"
    When I run quecto gateway for at least 5 seconds
    Then the mock LLM should not have received any requests containing "Check weather"

  # --- Cron job delivery ---

  Scenario: Gateway delivers cron job result to configured Telegram channel
    Given a cron job "report" with interval 2 seconds and message "Generate report" and deliver_to "telegram:12345"
    And a mock Telegram API
    And the mock LLM returns a text response "Daily report: all systems operational"
    When I run quecto gateway for at least 5 seconds
    Then the Telegram API should have received a sendMessage to chat "12345"
    And the sent message should contain "all systems operational"

  # --- Timeout handling ---

  Scenario: Gateway terminates cron job that exceeds timeout
    Given a cron job "slow-task" with interval 2 seconds and message "Slow task"
    And the mock LLM takes 30 seconds to respond
    And the config has cron exec_timeout of 3 seconds
    When I run quecto gateway for at least 10 seconds
    Then the cron job "slow-task" should have last_error containing "timeout"

  # --- LLM tool use in cron job ---

  Scenario: Cron job can use tools through the agent loop
    Given a cron job "disk-check" with interval 2 seconds and message "Check disk usage"
    And the mock LLM first returns a tool call for "exec" with args:
      | command | df -h |
    And the mock LLM then returns a text response "Disk usage checked"
    When I run quecto gateway for at least 5 seconds
    Then the mock LLM should have received at least 2 requests
