@done
Feature: Heartbeat (Periodic Autonomous Tasks)
  As a user
  I want Quecto to periodically check a task list and execute it
  So that routine tasks happen automatically

  Scenario: Parse simple tasks from HEARTBEAT.md content
    Given a HEARTBEAT.md with content:
      """
      - Check the weather forecast
      - Report current time
      """
    When the heartbeat content is parsed
    Then the parsed tasks should contain 2 items
    And task 1 should be "Check the weather forecast"
    And task 2 should be "Report current time"
    And no tasks should be marked as spawn

  Scenario: Parse spawn section marks tasks for subagent
    Given a HEARTBEAT.md with content:
      """
      ## Long Tasks (use spawn for async)
      - Search the web for AI news and summarize
      - Analyze monthly data
      """
    When the heartbeat content is parsed
    Then the parsed tasks should contain 2 items
    And task 1 should be marked as spawn
    And task 2 should be marked as spawn

  Scenario: Parse mixed sections with regular and spawn tasks
    Given a HEARTBEAT.md with content:
      """
      - Quick check
      ## Long Tasks (use spawn)
      - Slow analysis
      ## Regular
      - Another quick task
      """
    When the heartbeat content is parsed
    Then the parsed tasks should contain 3 items
    And task 1 should not be marked as spawn
    And task 2 should be marked as spawn
    And task 3 should not be marked as spawn

  Scenario: Load tasks from workspace file
    Given a workspace with a HEARTBEAT.md file containing:
      """
      - Check weather
      - Report time
      """
    When the heartbeat loads tasks from the workspace
    Then the parsed tasks should contain 2 items

  Scenario: Missing HEARTBEAT.md returns empty task list
    Given a workspace without a HEARTBEAT.md file
    When the heartbeat loads tasks from the workspace
    Then the parsed tasks should contain 0 items

  Scenario: Heartbeat result reports HEARTBEAT_OK on success
    Given a heartbeat result with 2 tasks found, 2 executed, and ok true
    Then the heartbeat status should be "HEARTBEAT_OK"

  Scenario: Heartbeat result reports HEARTBEAT_FAIL on failure
    Given a heartbeat result with 2 tasks found, 1 executed, and ok false
    Then the heartbeat status should be "HEARTBEAT_FAIL"

  @pending
  Scenario: Gateway heartbeat dispatches tasks at configured interval
    Given a running gateway with a mock LLM provider
    And a workspace HEARTBEAT.md containing:
      """
      - Check system health
      - Report disk usage
      """
    And the config has heartbeat_interval_minutes 5
    When the heartbeat tick fires
    Then the mock LLM should receive a request containing "Check system health"
    And the mock LLM should receive a request containing "Report disk usage"

  @pending
  Scenario: Heartbeat skips when no HEARTBEAT.md exists
    Given a running gateway with a mock LLM provider
    And no HEARTBEAT.md in the workspace
    And the config has heartbeat_interval_minutes 5
    When the heartbeat tick fires
    Then the mock LLM should not receive any requests

  @pending
  Scenario: Heartbeat spawn tasks use subagent
    Given a running gateway with a mock LLM provider
    And a workspace HEARTBEAT.md containing:
      """
      ## Long Tasks (use spawn)
      - Analyze monthly data
      """
    And the config has heartbeat_interval_minutes 5
    When the heartbeat tick fires
    Then the task "Analyze monthly data" should be dispatched via the spawn tool
