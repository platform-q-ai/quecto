@done
Feature: Subagent / Multi-Agent Architecture
  As an AI agent
  I want to spawn background subagents for complex tasks
  So that long-running work does not block the main conversation

  Scenario: Subagent context is created with a task
    Given a subagent spawn request with task "Summarize news"
    When the subagent context is created
    Then the subagent context should have task "Summarize news"
    And the subagent context should have an empty conversation history

  Scenario: Subagent inherits workspace restrictions
    Given a parent agent config with restrict_to_workspace true
    When a subagent context is created from the parent
    Then the subagent should also have restrict_to_workspace true

  Scenario: Subagent inherits workspace restrictions disabled
    Given a parent agent config with restrict_to_workspace false
    When a subagent context is created from the parent
    Then the subagent should also have restrict_to_workspace false

  Scenario: Spawn validates agent_id against allowlist
    Given an agent allowlist containing "news-bot" and "weather-bot"
    When I validate agent_id "news-bot"
    Then the validation should succeed

  Scenario: Spawn rejects disallowed agent_id
    Given an agent allowlist containing "news-bot" and "weather-bot"
    When I validate agent_id "evil-bot"
    Then the validation should fail with "not allowed"


