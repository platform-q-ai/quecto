@done
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
    Given a cron job file with name "weather-check" interval 2 and message "Check the weather"
    And a mock LLM that captures requests and returns text "Weather is sunny"
    When I run the quecto gateway subprocess for at least 5 seconds
    Then the captured LLM requests should contain "Check the weather"

  Scenario: Gateway executes multiple due cron jobs
    Given a cron job file with name "weather" interval 2 and message "Check weather"
    And a cron job file with name "backup" interval 2 and message "Run backup"
    And a mock LLM that captures requests and returns text "Task done"
    When I run the quecto gateway subprocess for at least 5 seconds
    Then the captured LLM requests should contain "Check weather"
    And the captured LLM requests should contain "Run backup"

  # --- Disabled jobs ---

  Scenario: Gateway skips disabled cron jobs
    Given a disabled cron job file with name "weather" interval 2 and message "Check weather"
    And a mock LLM that captures requests and returns text "Should not happen"
    When I run the quecto gateway subprocess for at least 5 seconds
    Then the captured LLM requests should not contain "Check weather"

  # --- No cron jobs ---

  Scenario: Gateway cron tick is a no-op when no jobs exist
    Given a mock LLM that captures requests and returns text "Should not happen"
    When I run the quecto gateway subprocess for at least 5 seconds
    Then the captured LLM requests should be empty
