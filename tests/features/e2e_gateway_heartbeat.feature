@pending
Feature: End-to-End Gateway Heartbeat Integration
  As a system operator
  I want the gateway to periodically execute heartbeat tasks
  So that routine maintenance tasks run automatically without user intervention

  The gateway event loop should include a heartbeat timer that fires at the
  configured interval, loads tasks from HEARTBEAT.md, and dispatches them
  through the agent. These tests verify the gateway actually wires the
  heartbeat into its event loop, not just that the application-layer function
  works in isolation.

  Background:
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server

  # --- Basic heartbeat firing ---

  Scenario: Gateway fires heartbeat and dispatches tasks to the LLM
    Given a workspace HEARTBEAT.md containing:
      """
      - Check system health
      """
    And the config has heartbeat enabled with interval 2 seconds
    And the mock LLM returns a text response "System healthy"
    When I run quecto gateway for at least 5 seconds
    Then the mock LLM should have received at least 1 request containing "Check system health"

  Scenario: Gateway fires heartbeat with multiple tasks
    Given a workspace HEARTBEAT.md containing:
      """
      - Check disk usage
      - Verify backups
      """
    And the config has heartbeat enabled with interval 2 seconds
    And the mock LLM returns a text response "All checks passed"
    When I run quecto gateway for at least 5 seconds
    Then the mock LLM should have received at least 1 request containing "Check disk usage"
    And the mock LLM should have received at least 1 request containing "Verify backups"

  # --- Disabled heartbeat ---

  Scenario: Gateway does not fire heartbeat when disabled in config
    Given a workspace HEARTBEAT.md containing:
      """
      - This should not run
      """
    And the config has heartbeat disabled
    And the mock LLM returns a text response "Should not happen"
    When I run quecto gateway for at least 5 seconds
    Then the mock LLM should not have received any heartbeat requests

  # --- Missing HEARTBEAT.md ---

  Scenario: Gateway heartbeat is a no-op when HEARTBEAT.md does not exist
    Given the config has heartbeat enabled with interval 2 seconds
    And the mock LLM returns a text response "Should not happen"
    When I run quecto gateway for at least 5 seconds
    Then the mock LLM should not have received any heartbeat requests

  # --- Spawn tasks ---

  Scenario: Gateway heartbeat dispatches spawn-marked tasks via subagent
    Given a workspace HEARTBEAT.md containing:
      """
      ## Long Tasks (use spawn)
      - Analyze monthly data
      """
    And the config has heartbeat enabled with interval 2 seconds
    And the mock LLM returns a text response "Analysis complete"
    When I run quecto gateway for at least 5 seconds
    Then the task "Analyze monthly data" should have been dispatched via spawn
