@done @tui
Feature: Master connection behind a feed task (N=1)
  As a TUI user (and the multi-session epic #1467 building on this seam)
  I want the master agent connection driven through the same fan-in feed-task
  pattern as sub-agent feeds (issue #1462)
  So that at N=1 nothing changes — frames, routing, and disconnect diagnosis
  are identical — while the event loop becomes independent of connection count

  @issue-1462
  Scenario: Master events delivered through the connection feed render identically
    Given a baseline frame from a master token handled directly
    And a fresh headless TUI harness
    When the same master token arrives through the master connection feed
    Then the frame should be identical to the direct-handling baseline

  @issue-1462
  Scenario: Stream close surfaces as a disconnect
    Given the TUI is connected to an agent with the left panel visible
    When the master connection's event stream closes
    Then the left panel should remain visible
    And the TUI should show a disconnect notification

  @issue-1462
  Scenario: A closed connection keeps the child exit diagnosis
    Given the TUI spawned its own agent child process
    When the agent child process aborts
    And the master connection's event stream closes
    Then the disconnect notification should include the child's exit detail
