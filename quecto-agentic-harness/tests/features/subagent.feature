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

  Scenario: Spawn accepts an allowed display label
    Given an agent allowlist containing "news-bot" and "weather-bot"
    When the parent requests a subagent named "news-bot"
    Then the validation should succeed

  Scenario: Spawn rejects a disallowed display label
    Given an agent allowlist containing "news-bot" and "weather-bot"
    When the parent requests a subagent named "evil-bot"
    Then the validation should fail with "not allowed"

  Scenario: Reusing a display label mints a new hidden identity
    Given a subagent named "worker" has exited
    When a parent spawns a subagent named "worker"
    Then the spawned subagent should have a new hidden identity

  Scenario: Reusing a display label starts with no inherited context
    Given a subagent named "worker" has exited
    When a parent spawns a subagent named "worker"
    Then the spawned subagent should have a clean conversation history

  Scenario: Live display labels are unique
    Given a live subagent named "worker"
    When a parent spawns a subagent named "worker"
    Then the spawn should fail with a duplicate display label error containing "worker"

  Scenario: Display labels target only live subagents
    Given a subagent named "worker" has exited
    When a parent tool targets display label "worker"
    Then the command should fail with no live subagent named "worker"


