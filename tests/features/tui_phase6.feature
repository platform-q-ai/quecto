@tui @pending
Feature: TUI Phase 6 — Session Tree, Forking, Compaction, and Auto-Retry
  Advanced session management and resilience features.

  Scenario: Session info displays stats
    Given a session info component with key "cli:default" and 10 messages
    When the component renders at width 60
    Then the rendered output should contain "cli:default"
    And the rendered output should contain "10"

  Scenario: Token stats format correctly
    Given token stats with input 15000 and output 3200
    When formatted for display
    Then the display should show "15k" and "3.2k"

  Scenario: Context bar color-codes by usage
    Given context usage at 85%
    When the context bar renders
    Then the percentage should be styled with warning color

  Scenario: Context bar shows red above 90%
    Given context usage at 95%
    When the context bar renders
    Then the percentage should be styled with error color

  Scenario: Retry indicator shows countdown
    Given a retry component with attempt 2 of 3 and delay 5 seconds
    When the component renders at width 60
    Then the rendered output should contain "2/3"
    And the rendered output should contain "5s"

  Scenario: Retry indicator is cancellable
    Given an active retry component
    When the user presses Escape
    Then the retry should be cancelled

  Scenario: Compaction indicator shows progress
    Given a compaction component with message "Auto-compacting..."
    When the component renders at width 60
    Then the rendered output should contain "Auto-compacting"

  Scenario: Message queue holds messages during compaction
    Given a message queue
    When messages "first" and "second" are queued
    Then the queue should contain 2 messages
    When the queue is drained
    Then the queue should be empty
