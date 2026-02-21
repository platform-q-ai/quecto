@pending
Feature: REPL Spawn Command
  As a user in the interactive REPL
  I want to spawn subagent tasks directly from the REPL
  So that I can delegate work to child agents during an interactive session

  The /spawn command runs a task as a child quecto process, waits for it
  to complete, and prints the result back into the REPL conversation.
  It can optionally target a named subagent profile (created via /agent create)
  or run with ad-hoc system prompts and model overrides.

  Background:
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server

  # --- Basic spawning ---

  Scenario: /spawn runs a task and prints the result
    Given the mock LLM returns a text response "The answer is 42"
    When I start quecto in REPL mode
    And I type "/spawn What is the meaning of life?"
    And I type "/exit"
    Then stdout should contain "The answer is 42"
    And a child quecto process should have been spawned

  Scenario: /spawn with named agent profile uses its system prompt
    Given a subagent profile "researcher" exists with system prompt "You are a researcher"
    And the mock LLM returns a text response "Research findings: quantum computing advances"
    When I start quecto in REPL mode
    And I type "/spawn --agent researcher What is new in quantum computing?"
    And I type "/exit"
    Then stdout should contain "quantum computing advances"
    And a child quecto process should have been spawned with system prompt "You are a researcher"

  Scenario: /spawn with inline system prompt
    Given the mock LLM returns a text response "Bonjour, le temps est beau"
    When I start quecto in REPL mode
    And I type "/spawn --system 'You are a French translator' Translate: the weather is nice"
    And I type "/exit"
    Then stdout should contain "Bonjour"
    And a child quecto process should have been spawned

  Scenario: /spawn with model override
    Given the mock LLM returns a text response "Quick response from mini"
    When I start quecto in REPL mode
    And I type "/spawn --model gpt-5-mini Summarize briefly"
    And I type "/exit"
    Then stdout should contain "Quick response from mini"

  # --- Error handling ---

  Scenario: /spawn with no task shows error
    When I start quecto in REPL mode
    And I type "/spawn"
    And I type "/exit"
    Then stdout should contain "missing task"

  Scenario: /spawn with nonexistent agent profile shows error
    When I start quecto in REPL mode
    And I type "/spawn --agent ghost Do something"
    And I type "/exit"
    Then stdout should contain "not found"

  Scenario: /spawn child failure is reported in REPL
    Given the mock LLM returns an HTTP 500 error
    When I start quecto in REPL mode
    And I type "/spawn Do something that will fail"
    And I type "/exit"
    Then stdout should contain "error"
    And the REPL should continue accepting input after the failure

  Scenario: /spawn child timeout is reported in REPL
    Given the mock LLM takes 10 seconds to respond
    When I start quecto in REPL mode
    And I type "/spawn --max-time 1 Slow task"
    And I type "/exit"
    Then stdout should contain "timed out"

  # --- Session isolation ---

  Scenario: /spawn uses an ephemeral session for the child
    Given the mock LLM returns a text response "Ephemeral child output"
    When I start quecto in REPL mode with flags "-s parent-session"
    And I type "/spawn Quick ephemeral task"
    And I type "/exit"
    Then the session "repl:parent-session" should not contain "Quick ephemeral task" as a user message
    And no child session files should exist

  # --- Result feeds back into parent conversation ---

  Scenario: /spawn result is available to the LLM in the next turn
    Given the mock LLM returns sequential responses:
      | Spawn result: AI trends show growth in agents |
      | Based on the research, agents are the future   |
    When I start quecto in REPL mode
    And I type "/spawn Research AI trends"
    And I type "What did the research find?"
    And I type "/exit"
    Then stdout should contain "agents are the future"

  # --- Help ---

  Scenario: /help includes /spawn in the command list
    When I start quecto in REPL mode
    And I type "/help"
    And I type "/exit"
    Then stdout should contain "/spawn"

  Scenario: /spawn --help shows usage
    When I start quecto in REPL mode
    And I type "/spawn --help"
    And I type "/exit"
    Then stdout should contain "Usage: /spawn"
    And stdout should contain "--agent"
    And stdout should contain "--system"
    And stdout should contain "--model"
    And stdout should contain "--max-time"
