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

  @pending
  Scenario: Job executes at scheduled time
    Given a scheduled job with interval 1 second
    When I wait for 2 seconds
    Then the job should have executed at least once

  @pending
  Scenario: Job execution respects timeout
    Given a scheduled job with exec_timeout_minutes 1
    When the job executes
    Then the job should be terminated after the timeout

  @pending
  Scenario: Job delivers result to a channel
    Given a scheduled job configured to deliver to Telegram chat "12345"
    When the job executes
    Then the result should be sent to Telegram chat "12345"
