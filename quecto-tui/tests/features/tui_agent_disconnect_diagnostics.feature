@done @tui
Feature: Agent disconnect keeps context and reports diagnostics
  As a TUI user whose agent process dies mid-session (e.g. a panic-abort near a
  full context window, issue #1047)
  I want the TUI to keep the left panel visible and tell me WHY the agent went away
  So that the session does not silently look "disconnected" with no way to diagnose it

  @issue-1047
  Scenario: Left panel remains visible after the agent disconnects
    Given the TUI is connected to an agent with the left panel visible
    When the agent connection closes unexpectedly
    Then the left panel should remain visible
    And the TUI should show a disconnect notification

  @issue-1047
  Scenario: Disconnect notification reports the agent child's exit detail
    Given the TUI spawned its own agent child process
    When the agent child process aborts with a signal
    Then the disconnect notification should include the child's exit detail
