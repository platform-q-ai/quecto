@done
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
    Given a HEARTBEAT.md in the e2e workspace containing:
      """
      - Check system health
      """
    And the config has heartbeat enabled with interval 2 seconds
    And a mock LLM that captures requests and returns text "System healthy"
    When I run the quecto gateway subprocess for at least 5 seconds
    Then the captured LLM requests should contain "Check system health"

  Scenario: Gateway fires heartbeat with multiple tasks
    Given a HEARTBEAT.md in the e2e workspace containing:
      """
      - Check disk usage
      - Verify backups
      """
    And the config has heartbeat enabled with interval 2 seconds
    And a mock LLM that captures requests and returns text "All checks passed"
    When I run the quecto gateway subprocess for at least 5 seconds
    Then the captured LLM requests should contain "Check disk usage"
    And the captured LLM requests should contain "Verify backups"

  # --- Disabled heartbeat ---

  Scenario: Gateway does not fire heartbeat when disabled in config
    Given a HEARTBEAT.md in the e2e workspace containing:
      """
      - This should not run
      """
    And the config has heartbeat disabled
    And a mock LLM that captures requests and returns text "Should not happen"
    When I run the quecto gateway subprocess for at least 5 seconds
    Then the captured LLM requests should not contain "This should not run"

  # --- Missing HEARTBEAT.md ---

  Scenario: Gateway heartbeat is a no-op when HEARTBEAT.md does not exist
    Given the config has heartbeat enabled with interval 2 seconds
    And a mock LLM that captures requests and returns text "Should not happen"
    When I run the quecto gateway subprocess for at least 5 seconds
    Then the captured LLM requests should be empty

  # --- Spawn tasks ---

  Scenario: Gateway heartbeat dispatches spawn-marked tasks via subagent
    Given a HEARTBEAT.md in the e2e workspace containing:
      """
      ## Long Tasks (use spawn)
      - Analyze monthly data
      """
    And the config has heartbeat enabled with interval 2 seconds
    And a mock LLM that captures requests and returns text "Analysis complete"
    When I run the quecto gateway subprocess for at least 5 seconds
    Then the captured LLM requests should contain "Analyze monthly data"
    And the captured LLM requests should contain "Spawn"
