@done
Feature: Scheduled Tasks (Cron)
  As a user
  I want to schedule one-time and recurring tasks
  So that Quecto can perform actions on a schedule

  Scenario: Add a job with interval schedule
    Given a cron store
    When I add a job "Check weather" with interval 3600 seconds
    Then the job "Check weather" should exist in the store
    And the job should be enabled

  Scenario: Add a job with cron expression
    Given a cron store
    When I add a job "Morning brief" with cron expression "0 9 * * *"
    Then the job "Morning brief" should exist in the store

  Scenario: List all jobs
    Given a cron store
    And a job "Check weather" with interval 3600 seconds exists
    And a job "Morning brief" with cron expression "0 9 * * *" exists
    When I list all jobs
    Then the job list should contain 2 jobs
    And the job list should include "Check weather"
    And the job list should include "Morning brief"

  Scenario: Remove a job
    Given a cron store
    And a job "Check weather" with interval 3600 seconds exists
    When I remove the job "Check weather"
    Then the job "Check weather" should not exist in the store

  Scenario: Disable a job
    Given a cron store
    And a job "Check weather" with interval 3600 seconds exists
    When I disable the job "Check weather"
    Then the job "Check weather" should be disabled

  Scenario: Enable a disabled job
    Given a cron store
    And a disabled job "Check weather" with interval 3600 seconds exists
    When I enable the job "Check weather"
    Then the job "Check weather" should be enabled

  Scenario: Jobs persist across store instances
    Given a cron store
    And a job "Check weather" with interval 3600 seconds exists
    When the cron store is recreated from the same directory
    Then the job "Check weather" should exist in the store

  @done
  Scenario: Gateway executes interval job when due
    Given a running gateway with a mock LLM provider
    And a cron job "weather" with interval 2 seconds and message "Check weather"
    When the cron tick fires
    Then the mock LLM should receive a request containing "Check weather"

  @done
  Scenario: Gateway skips disabled cron jobs
    Given a running gateway with a mock LLM provider
    And a disabled cron job "weather" with interval 2 seconds
    When the cron tick fires
    Then the mock LLM should not receive any requests

  @done
  Scenario: Cron job execution respects timeout
    Given a running gateway with a mock LLM provider
    And a cron job "slow-task" with interval 2 seconds and message "Run slow task"
    And the config has exec_timeout_minutes 1
    When the cron job starts executing and exceeds the timeout
    Then the job execution should be terminated
    And the job should be marked as last_error containing "timeout"

  @done
  Scenario: Cron job delivers result to configured channel
    Given a running gateway with a mock LLM provider
    And a mock Telegram API
    And a cron job "report" with interval 60 seconds and deliver_to "telegram:12345"
    And the gateway agent responds with "Daily report: all systems operational"
    When the cron tick fires for job "report"
    Then the Telegram API should receive a sendMessage to chat "12345"
    And the message should contain "all systems operational"

  @done @pr2-correctness
  Scenario: Gateway skips cron-expression jobs until parsing is implemented
    Given a running gateway with a mock LLM provider
    And a cron job "morning-brief" with cron expression "0 9 * * *" and message "Good morning brief"
    When the cron tick fires
    Then the mock LLM should not receive any requests
    And the job should be marked as last_error containing "not implemented"

  # --- Issue #105: Run-once cron jobs ---

  Scenario: Add a run-once job via cron tool
    Given a cron store
    When I add a run-once job "dentist-reminder" with interval 1800 seconds and message "Call the dentist"
    Then the job "dentist-reminder" should exist in the store
    And the job "dentist-reminder" should be marked as run_once

  Scenario: Run-once job is auto-removed after execution
    Given a running gateway with a mock LLM provider
    And a run-once cron job "reminder" with interval 1 seconds and message "Reminder: check deployment"
    When the cron tick fires
    Then the mock LLM should receive a request containing "Reminder: check deployment"
    And the gateway job "reminder" should not exist in the store

  Scenario: Regular recurring job is NOT removed after execution
    Given a running gateway with a mock LLM provider
    And a cron job "hourly" with interval 1 seconds and message "Hourly check"
    When the cron tick fires
    Then the mock LLM should receive a request containing "Hourly check"
    And the gateway job "hourly" should still exist in the store

  Scenario: Run-once field persists across store instances
    Given a cron store
    And a run-once job "one-shot" with interval 3600 seconds exists
    When the cron store is recreated from the same directory
    Then the job "one-shot" should exist in the store
    And the job "one-shot" should be marked as run_once

  Scenario: Run-once job list output shows one-shot indicator
    Given a cron store
    And a run-once job "reminder" with interval 1800 seconds exists
    When I list all jobs via the cron tool
    Then the list output should contain "one-shot"

  # --- Issue #106: Cron job result delivery ---

  Scenario: Cron job result is delivered to deliver_to channel
    Given a running gateway with a mock LLM provider and outbound channel
    And a cron job "report" with interval 1 seconds and message "Generate report" and deliver_to "telegram:12345"
    And the gateway agent responds with "Daily report complete"
    When the cron tick fires and results are delivered
    Then the outbound channel should have received a message to "telegram:12345"
    And the outbound message should contain "Daily report complete"

  Scenario: Cron job without deliver_to does not send outbound message
    Given a running gateway with a mock LLM provider and outbound channel
    And a cron job "internal" with interval 1 seconds and message "Internal task"
    When the cron tick fires and results are delivered
    Then the outbound channel should not have received any messages
