@done @tui
Feature: Master connection behind a feed task (N=1)
  As a TUI user (and the multi-session epic #1467 building on this seam)
  I want the master agent connection driven through the same fan-in feed-task
  pattern as sub-agent feeds (issue #1462)
  So that at N=1 nothing changes — frames, routing, and disconnect diagnosis
  are identical — while the event loop becomes independent of connection count

  @issue-1462
  Scenario: Master events delivered through the fan-in render identically
    Given a headless TUI harness showing a master token via direct handling
    When the same master token is delivered through the fan-in tagged with the master tab source
    Then the fan-in frame should be identical to the directly handled frame

  @issue-1462
  Scenario: Stream close arrives as an explicit Closed sentinel
    Given the TUI is connected to an agent with the left panel visible
    When the master connection delivers its Closed sentinel
    Then the left panel should remain visible
    And the TUI should show a disconnect notification

  @issue-1462
  Scenario: The Closed sentinel keeps the child exit diagnosis
    Given the TUI spawned its own agent child process
    When the agent child process aborts and the Closed sentinel is delivered
    Then the disconnect notification should include the child's exit detail
