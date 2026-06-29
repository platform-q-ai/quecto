@tui @pending
Feature: Auto-remove exited subagent status bars after delay (#540)
  As a TUI user
  I want exited subagent status bars to disappear automatically
  So that stale entries don't clutter the interface

  Scenario: Exited subagent bar is removed after grace period
    Given a subagent "worker-1" with status "exited"
    And the exited_at timestamp is 6 seconds ago
    When the GC tick fires
    Then the subagent bar for "worker-1" should be removed

  Scenario: Exited subagent bar is kept during grace period
    Given a subagent "worker-1" with status "exited"
    And the exited_at timestamp is 2 seconds ago
    When the GC tick fires
    Then the subagent bar for "worker-1" should still be visible

  Scenario: Running subagent bar is never auto-removed
    Given a subagent "worker-1" with status "running"
    When the GC tick fires
    Then the subagent bar for "worker-1" should still be visible

  Scenario: Transition to exited records timestamp
    Given a subagent "worker-1" with status "running"
    When the subagent status changes to "exited"
    Then an exited_at timestamp should be recorded
