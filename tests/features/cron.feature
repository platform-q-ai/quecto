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

  @done
  Scenario: Gateway executes cron-expression job when current time matches
    Given a running gateway with a mock LLM provider
    And a cron job "every-minute" with cron expression "* * * * *" and message "Run every minute"
    When the cron tick fires
    Then the mock LLM should receive a request containing "Run every minute"

  @done
  Scenario: Cron job has created_at timestamp set at creation time
    Given a cron store
    When I add a job "Timestamped" with interval 3600 seconds
    Then the job "Timestamped" should have a non-zero created_at

  @done
  Scenario: Cron tool list output includes diagnostics
    Given a running gateway with a mock LLM provider
    And a cron job "weather" with interval 60 seconds and message "Check weather"
    When the cron tick fires
    And I list jobs via the cron tool
    Then the cron tool list output should contain "last_run"
    And the cron tool list output should contain "created"

  @done
  Scenario: Cron tool list output shows last_error when present
    Given a cron store
    And a job "Broken" with interval 60 seconds exists
    And the job "Broken" has last_error "timeout"
    When I list jobs via the cron tool
    Then the cron tool list output should contain "last_error"
    And the cron tool list output should contain "timeout"

  @done
  Scenario: Adding a job with invalid deliver_to is rejected
    Given a cron store
    When I try to add a job "Bad" with interval 60 seconds and deliver_to "current"
    Then the cron tool should return an error containing "invalid deliver_to"

  @done
  Scenario: Adding a job with valid deliver_to succeeds
    Given a cron store
    When I try to add a job "Good" with interval 60 seconds and deliver_to "telegram:12345"
    Then the cron tool should return success
