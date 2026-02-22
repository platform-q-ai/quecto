@done
Feature: REPL Heartbeat Management
  As a user in the interactive REPL
  I want to view and edit the heartbeat task list and toggle heartbeat on/off
  So that I can control periodic autonomous tasks without editing files manually

  The REPL should support /heartbeat slash commands that operate on the
  HEARTBEAT.md file in the workspace and the heartbeat config section.
  Changes should take effect on the next gateway heartbeat tick.

  Background:
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server

  # --- Viewing ---

  Scenario: /heartbeat show displays current tasks
    Given a workspace HEARTBEAT.md containing:
      """
      - Check system health
      - Report disk usage
      """
    When I start quecto in REPL mode
    And I type "/heartbeat show"
    And I type "/exit"
    Then stdout should contain "Check system health"
    And stdout should contain "Report disk usage"
    And stdout should contain "2 tasks"

  Scenario: /heartbeat show when no HEARTBEAT.md exists
    When I start quecto in REPL mode
    And I type "/heartbeat show"
    And I type "/exit"
    Then stdout should contain "No heartbeat tasks configured"

  Scenario: /heartbeat show displays spawn-marked tasks differently
    Given a workspace HEARTBEAT.md containing:
      """
      - Quick check
      ## Long Tasks (use spawn)
      - Analyze data
      """
    When I start quecto in REPL mode
    And I type "/heartbeat show"
    And I type "/exit"
    Then stdout should contain "Quick check"
    And stdout should contain "Analyze data"
    And stdout should contain "spawn"

  # --- Adding tasks ---

  Scenario: /heartbeat add appends a new task
    Given a workspace HEARTBEAT.md containing:
      """
      - Check system health
      """
    When I start quecto in REPL mode
    And I type "/heartbeat add Report memory usage"
    And I type "/exit"
    Then the workspace HEARTBEAT.md should contain "Check system health"
    And the workspace HEARTBEAT.md should contain "Report memory usage"

  Scenario: /heartbeat add creates HEARTBEAT.md if it does not exist
    When I start quecto in REPL mode
    And I type "/heartbeat add First task ever"
    And I type "/exit"
    Then the workspace HEARTBEAT.md should exist
    And the workspace HEARTBEAT.md should contain "First task ever"

  Scenario: /heartbeat add --spawn appends a task under a spawn section
    When I start quecto in REPL mode
    And I type "/heartbeat add --spawn Analyze monthly data"
    And I type "/exit"
    Then the workspace HEARTBEAT.md should contain "Analyze monthly data"
    And the workspace HEARTBEAT.md should contain "spawn"

  Scenario: /heartbeat add with no task text shows error
    When I start quecto in REPL mode
    And I type "/heartbeat add"
    And I type "/exit"
    Then stdout should contain "missing task description"

  # --- Removing tasks ---

  Scenario: /heartbeat remove deletes a task by text match
    Given a workspace HEARTBEAT.md containing:
      """
      - Check system health
      - Report disk usage
      """
    When I start quecto in REPL mode
    And I type "/heartbeat remove Check system health"
    And I type "/exit"
    Then the workspace HEARTBEAT.md should not contain "Check system health"
    And the workspace HEARTBEAT.md should contain "Report disk usage"

  Scenario: /heartbeat remove nonexistent task shows error
    Given a workspace HEARTBEAT.md containing:
      """
      - Check system health
      """
    When I start quecto in REPL mode
    And I type "/heartbeat remove Ghost task"
    And I type "/exit"
    Then stdout should contain "not found"

  # --- Enabling/disabling ---

  Scenario: /heartbeat disable turns off heartbeat in config
    Given the config has heartbeat enabled with interval 30 seconds
    When I start quecto in REPL mode
    And I type "/heartbeat disable"
    And I type "/exit"
    Then stdout should contain "Heartbeat disabled"
    And the config file should have heartbeat enabled set to false

  Scenario: /heartbeat enable turns on heartbeat in config
    Given the config has heartbeat disabled
    When I start quecto in REPL mode
    And I type "/heartbeat enable"
    And I type "/exit"
    Then stdout should contain "Heartbeat enabled"
    And the config file should have heartbeat enabled set to true

  Scenario: /heartbeat interval sets the heartbeat interval
    When I start quecto in REPL mode
    And I type "/heartbeat interval 300"
    And I type "/exit"
    Then stdout should contain "Heartbeat interval set to 300s"
    And the config file should have heartbeat interval set to 300

  Scenario: /heartbeat interval with invalid value shows error
    When I start quecto in REPL mode
    And I type "/heartbeat interval abc"
    And I type "/exit"
    Then stdout should contain "invalid interval"

  # --- Status ---

  Scenario: /heartbeat status shows current heartbeat configuration
    Given the config has heartbeat enabled with interval 60 seconds
    And a workspace HEARTBEAT.md containing:
      """
      - Check health
      """
    When I start quecto in REPL mode
    And I type "/heartbeat status"
    And I type "/exit"
    Then stdout should contain "enabled"
    And stdout should contain "60s"
    And stdout should contain "1 task"

  # --- Help ---

  Scenario: /heartbeat with no subcommand shows usage
    When I start quecto in REPL mode
    And I type "/heartbeat"
    And I type "/exit"
    Then stdout should contain "Usage: /heartbeat"
    And stdout should contain "show"
    And stdout should contain "add"
    And stdout should contain "remove"

  Scenario: /help includes /heartbeat in the command list
    When I start quecto in REPL mode
    And I type "/help"
    And I type "/exit"
    Then stdout should contain "/heartbeat"
