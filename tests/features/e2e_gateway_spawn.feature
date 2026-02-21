@done
Feature: End-to-End Spawn Tool via Gateway
  As a user interacting through Telegram
  I want the agent to spawn subagents when it needs to delegate work
  So that complex tasks are handled by specialized child agents

  When the LLM calls the spawn tool during a gateway session, the gateway
  should execute a child quecto process, collect its output, and feed the
  result back into the parent agent's conversation. These tests verify the
  full spawn lifecycle through the running gateway, not just SpawnTool in
  isolation.

  Background:
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And a mock Telegram API that supports voice downloads

  # --- Basic spawn through gateway ---

  Scenario: LLM spawns a subagent via tool call during gateway session
    Given the mock LLM first returns a tool call for "spawn" with args:
      | task     | Research the weather forecast |
      | agent_id | weather-agent                |
    And the mock LLM then returns a text response "The weather agent reported sunny skies"
    When user "12345" sends text "What is the weather?" via Telegram to the running gateway
    Then the gateway Telegram mock should have received a sendMessage containing "sunny skies"
    And the child session "cli:weather-agent" should exist in the base directory

  # --- Spawn with system prompt ---

  Scenario: LLM spawns a subagent with custom system prompt
    Given the mock LLM first returns a tool call for "spawn" with args:
      | task   | Summarize the news           |
      | system | You are a news summarizer    |
    And the mock LLM then returns a text response "Here is the summary"
    When user "12345" sends text "Summarize today's news" via Telegram to the running gateway
    Then the gateway Telegram mock should have received a sendMessage containing "summary"

  # --- Spawn failure handling ---

  Scenario: Spawn tool failure is reported back to the parent LLM
    Given the mock LLM first returns a tool call for "spawn" with args:
      | task     | Do something impossible |
      | agent_id | failing-agent           |
    And the mock LLM then returns a text response "The subagent failed, I will try another approach"
    When user "12345" sends text "Do the impossible" via Telegram to the running gateway
    Then the gateway Telegram mock should have received a sendMessage containing "another approach"

  # --- Spawn isolation ---

  Scenario: Spawned subagent uses a separate session from parent
    Given the mock LLM first returns a tool call for "spawn" with args:
      | task     | Independent research    |
      | agent_id | research-child          |
    And the mock LLM then returns a text response "Research complete"
    When user "12345" sends text "Start research" via Telegram to the running gateway
    Then the child session "cli:research-child" should exist in the base directory
