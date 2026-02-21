@pending
Feature: REPL Cron Job Management
  As a user in the interactive REPL
  I want to create, list, modify, and remove scheduled cron jobs
  So that I can manage recurring tasks without leaving the conversation

  The REPL should support /cron slash commands that operate on the same
  FileCronStore used by the gateway. Jobs created in the REPL should be
  picked up by the gateway's cron tick when it runs.

  Background:
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server

  # --- Listing ---

  Scenario: /cron list shows no jobs when none exist
    When I start quecto in REPL mode
    And I type "/cron list"
    And I type "/exit"
    Then stdout should contain "No scheduled jobs"

  Scenario: /cron list shows existing jobs
    Given a cron job "weather" with interval 3600 seconds already exists on disk
    When I start quecto in REPL mode
    And I type "/cron list"
    And I type "/exit"
    Then stdout should contain "weather"
    And stdout should contain "3600"

  # --- Adding jobs ---

  Scenario: /cron add creates an interval-based job
    When I start quecto in REPL mode
    And I type "/cron add weather --interval 3600 --message Check the weather"
    And I type "/exit"
    Then stdout should contain "Job 'weather' created"
    And the cron store should contain a job named "weather"
    And the job "weather" should have interval 3600

  Scenario: /cron add creates a cron-expression job
    When I start quecto in REPL mode
    And I type "/cron add morning-brief --cron '0 9 * * *' --message Good morning brief"
    And I type "/exit"
    Then stdout should contain "Job 'morning-brief' created"
    And the cron store should contain a job named "morning-brief"

  Scenario: /cron add with deliver_to sets delivery target
    When I start quecto in REPL mode
    And I type "/cron add report --interval 86400 --message Daily report --deliver-to telegram:12345"
    And I type "/exit"
    Then stdout should contain "Job 'report' created"
    And the job "report" should have deliver_to "telegram:12345"

  Scenario: /cron add with missing message shows error
    When I start quecto in REPL mode
    And I type "/cron add bad-job --interval 60"
    And I type "/exit"
    Then stdout should contain "missing required flag: --message"

  Scenario: /cron add with missing schedule shows error
    When I start quecto in REPL mode
    And I type "/cron add bad-job --message Check something"
    And I type "/exit"
    Then stdout should contain "missing schedule: specify --interval or --cron"

  Scenario: /cron add with duplicate name shows error
    Given a cron job "weather" with interval 3600 seconds already exists on disk
    When I start quecto in REPL mode
    And I type "/cron add weather --interval 60 --message Another weather check"
    And I type "/exit"
    Then stdout should contain "already exists"

  # --- Removing jobs ---

  Scenario: /cron remove deletes a job
    Given a cron job "weather" with interval 3600 seconds already exists on disk
    When I start quecto in REPL mode
    And I type "/cron remove weather"
    And I type "/exit"
    Then stdout should contain "Job 'weather' removed"
    And the cron store should not contain a job named "weather"

  Scenario: /cron remove nonexistent job shows error
    When I start quecto in REPL mode
    And I type "/cron remove ghost"
    And I type "/exit"
    Then stdout should contain "not found"

  # --- Enabling/disabling ---

  Scenario: /cron disable stops a job from running
    Given a cron job "weather" with interval 3600 seconds already exists on disk
    When I start quecto in REPL mode
    And I type "/cron disable weather"
    And I type "/exit"
    Then stdout should contain "Job 'weather' disabled"
    And the job "weather" should be disabled in the cron store

  Scenario: /cron enable re-enables a disabled job
    Given a disabled cron job "weather" with interval 3600 seconds already exists on disk
    When I start quecto in REPL mode
    And I type "/cron enable weather"
    And I type "/exit"
    Then stdout should contain "Job 'weather' enabled"
    And the job "weather" should be enabled in the cron store

  # --- Help ---

  Scenario: /cron with no subcommand shows usage
    When I start quecto in REPL mode
    And I type "/cron"
    And I type "/exit"
    Then stdout should contain "Usage: /cron"
    And stdout should contain "add"
    And stdout should contain "list"
    And stdout should contain "remove"

  Scenario: /help includes /cron in the command list
    When I start quecto in REPL mode
    And I type "/help"
    And I type "/exit"
    Then stdout should contain "/cron"

  # --- Persistence across REPL sessions ---

  Scenario: Cron job created in REPL persists across REPL restarts
    When I start quecto in REPL mode
    And I type "/cron add persist-test --interval 120 --message Test persistence"
    And I type "/exit"
    When I start quecto in REPL mode
    And I type "/cron list"
    And I type "/exit"
    Then stdout should contain "persist-test"
