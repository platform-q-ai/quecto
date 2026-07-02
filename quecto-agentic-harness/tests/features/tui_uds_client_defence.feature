Feature: TUI UDS client defensive bounds
  As a TUI user connected to an agent over UDS
  I want the client to bound untrusted input while preserving normal event handling
  So that a buggy or malicious agent cannot waste memory or disrupt later events

  @issue-982 @tui
  Scenario: Oversized agent events are discarded without disrupting later events
    Given the TUI is connected to an agent event stream
    When the agent sends an event larger than the supported event size followed by a valid token event
    Then the TUI should ignore the oversized event
    And the TUI should receive the later token event

  @issue-982 @tui
  Scenario: Agent events just below the size limit are handled normally
    Given the TUI is connected to an agent event stream
    When the agent sends an event just below the supported event size limit
    Then the TUI should receive the event

  @issue-982 @tui
  Scenario: Completion events keep their observable behaviour
    Given the TUI is connected to an agent event stream
    When the agent reports completion with details the TUI does not display
    Then completion is shown as before

  @issue-982 @tui
  Scenario: Undisplayed completion details are discarded
    Given the TUI is connected to an agent event stream
    When the agent reports completion with details the TUI does not display
    Then undisplayed completion details do not remain in the client event
