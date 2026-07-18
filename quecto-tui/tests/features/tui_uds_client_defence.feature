@done
Feature: TUI UDS client defensive bounds
  As a TUI user connected to an agent over UDS
  I want the client to bound untrusted input while preserving normal event handling
  So that a buggy or malicious agent cannot waste memory or disrupt later events

  @issue-982 @issue-1016 @tui
  Scenario: Oversized agent events are discarded without disrupting later events
    Given the TUI is connected to an agent event stream
    When the agent sends an event larger than the supported event size followed by a valid token event
    Then the TUI should ignore the oversized event
    And the TUI should receive the later token event

  @issue-982 @issue-1016 @issue-1112 @tui
  Scenario: Agent events just below the size limit are handled normally
    Given the TUI is connected to an agent event stream
    When the agent sends an event just below the supported event size limit
    Then the TUI should receive the event
    And the TUI emits no warning log

  @issue-1016 @tui
  Scenario: Repeated large agent events do not disrupt later events
    Given the TUI is connected to an agent event stream
    When the agent sends repeated oversized events followed by a valid token event
    Then the TUI should ignore the oversized events
    And the TUI should receive the later token event

  @issue-1047 @issue-1062 @adr-0008-part4 @tui
  Scenario: Oversized agent event drops are reported by the client
    Given the TUI is connected to an agent event stream
    When the agent sends an event larger than the supported event size
    And the agent then sends a valid token event
    Then the TUI reports one oversized agent event was dropped
    And the TUI should receive the later token event

  @issue-1112 @tui
  Scenario: Oversized agent event drops are warning-logged for diagnostics
    Given the TUI is connected to an agent event stream
    When the agent sends an event larger than the supported event size
    And the agent then sends a valid token event
    Then the TUI emits a warning log for the dropped oversized event

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
