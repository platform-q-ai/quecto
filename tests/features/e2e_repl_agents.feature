@done
Feature: REPL Subagent Management
  As a user in the interactive REPL
  I want to create, list, edit, run, and remove named subagent profiles
  So that I can define reusable specialist agents and invoke them on demand

  Subagent profiles are stored on disk as configuration files in the workspace.
  Each profile has a name, system prompt, optional model override, and optional
  allowed tools. The /agent commands manage these profiles, and /spawn dispatches
  a task to a named profile.

  Background:
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server

  # --- Listing ---

  Scenario: /agent list shows no agents when none exist
    When I start quecto in REPL mode
    And I type "/agent list"
    And I type "/exit"
    Then stdout should contain "No subagent profiles configured"

  Scenario: /agent list shows existing agent profiles
    Given a subagent profile "researcher" exists with system prompt "You are a researcher"
    And a subagent profile "coder" exists with system prompt "You are a coder"
    When I start quecto in REPL mode
    And I type "/agent list"
    And I type "/exit"
    Then stdout should contain "researcher"
    And stdout should contain "coder"

  # --- Creating ---

  Scenario: /agent create defines a new subagent profile
    When I start quecto in REPL mode
    And I type "/agent create researcher --system You are a research specialist"
    And I type "/exit"
    Then stdout should contain "Agent 'researcher' created"
    And a subagent profile "researcher" should exist on disk
    And the profile "researcher" should have system prompt "You are a research specialist"

  Scenario: /agent create with model override
    When I start quecto in REPL mode
    And I type "/agent create fast-bot --system Quick answers only --model gpt-5-mini"
    And I type "/exit"
    Then stdout should contain "Agent 'fast-bot' created"
    And the profile "fast-bot" should have model "gpt-5-mini"

  Scenario: /agent create with duplicate name shows error
    Given a subagent profile "researcher" exists with system prompt "You are a researcher"
    When I start quecto in REPL mode
    And I type "/agent create researcher --system Another researcher"
    And I type "/exit"
    Then stdout should contain "already exists"

  Scenario: /agent create with missing system prompt shows error
    When I start quecto in REPL mode
    And I type "/agent create nameless"
    And I type "/exit"
    Then stdout should contain "missing required flag: --system"

  Scenario: /agent create with invalid name shows error
    When I start quecto in REPL mode
    And I type "/agent create ../escape --system Malicious"
    And I type "/exit"
    Then stdout should contain "invalid agent name"

  # --- Viewing ---

  Scenario: /agent show displays a profile's configuration
    Given a subagent profile "researcher" exists with system prompt "You are a research specialist" and model "gpt-5"
    When I start quecto in REPL mode
    And I type "/agent show researcher"
    And I type "/exit"
    Then stdout should contain "researcher"
    And stdout should contain "You are a research specialist"
    And stdout should contain "gpt-5"

  Scenario: /agent show nonexistent profile shows error
    When I start quecto in REPL mode
    And I type "/agent show ghost"
    And I type "/exit"
    Then stdout should contain "not found"

  # --- Editing ---

  Scenario: /agent edit updates the system prompt
    Given a subagent profile "researcher" exists with system prompt "Old prompt"
    When I start quecto in REPL mode
    And I type "/agent edit researcher --system New and improved prompt"
    And I type "/exit"
    Then stdout should contain "Agent 'researcher' updated"
    And the profile "researcher" should have system prompt "New and improved prompt"

  Scenario: /agent edit updates the model
    Given a subagent profile "researcher" exists with system prompt "You are a researcher"
    When I start quecto in REPL mode
    And I type "/agent edit researcher --model gpt-5-mini"
    And I type "/exit"
    Then stdout should contain "Agent 'researcher' updated"
    And the profile "researcher" should have model "gpt-5-mini"

  Scenario: /agent edit nonexistent profile shows error
    When I start quecto in REPL mode
    And I type "/agent edit ghost --system Something"
    And I type "/exit"
    Then stdout should contain "not found"

  # --- Removing ---

  Scenario: /agent remove deletes a profile
    Given a subagent profile "researcher" exists with system prompt "You are a researcher"
    When I start quecto in REPL mode
    And I type "/agent remove researcher"
    And I type "/exit"
    Then stdout should contain "Agent 'researcher' removed"
    And a subagent profile "researcher" should not exist on disk

  Scenario: /agent remove nonexistent profile shows error
    When I start quecto in REPL mode
    And I type "/agent remove ghost"
    And I type "/exit"
    Then stdout should contain "not found"

  # --- Running ---

  Scenario: /agent run dispatches a task to a named profile
    Given a subagent profile "researcher" exists with system prompt "You are a researcher"
    And the mock LLM returns a text response "Research complete: AI is advancing"
    When I start quecto in REPL mode
    And I type "/agent run researcher Research the latest AI trends"
    And I type "/exit"
    Then stdout should contain "Research complete"
    And a child quecto process should have been spawned with system prompt "You are a researcher"

  Scenario: /agent run with nonexistent profile shows error
    When I start quecto in REPL mode
    And I type "/agent run ghost Do something"
    And I type "/exit"
    Then stdout should contain "not found"

  Scenario: /agent run with no task shows error
    Given a subagent profile "researcher" exists with system prompt "You are a researcher"
    When I start quecto in REPL mode
    And I type "/agent run researcher"
    And I type "/exit"
    Then stdout should contain "missing task"

  # --- Help ---

  Scenario: /agent with no subcommand shows usage
    When I start quecto in REPL mode
    And I type "/agent"
    And I type "/exit"
    Then stdout should contain "Usage: /agent"
    And stdout should contain "create"
    And stdout should contain "list"
    And stdout should contain "show"
    And stdout should contain "edit"
    And stdout should contain "remove"
    And stdout should contain "run"

  Scenario: /help includes /agent in the command list
    When I start quecto in REPL mode
    And I type "/help"
    And I type "/exit"
    Then stdout should contain "/agent"

  # --- Persistence ---

  Scenario: Agent profile created in REPL persists across restarts
    When I start quecto in REPL mode
    And I type "/agent create persist-bot --system Persistent agent"
    And I type "/exit"
    When I start quecto in REPL mode
    And I type "/agent list"
    And I type "/exit"
    Then stdout should contain "persist-bot"
