@tui @pending
Feature: TUI Phase 8 — Extension System and UDS Tool Registration
  TUI-registered tools, widgets, and replaceable components.

  Scenario: TUI registers a tool with the agent
    Given a tool registration for "tui_confirm" with description "Show confirm dialog"
    When serialized as a register_tools command
    Then the JSON should contain tool name "tui_confirm"

  Scenario: TUI handles execute_tool event
    Given an execute_tool event for "tui_confirm" with call ID "tc-1"
    When the event is dispatched
    Then a tool result should be queued for call ID "tc-1"

  Scenario: Widget renders above editor
    Given a widget "status" with content "Build: OK"
    When the widget container renders at width 40
    Then the rendered output should contain "Build: OK"

  Scenario: Widget can be cleared
    Given a widget "status" with content "Build: OK"
    When the widget is cleared
    Then the widget container should render empty

  Scenario: Multiple widgets render in order
    Given widget "a" with content "First"
    And widget "b" with content "Second"
    When the widget container renders at width 40
    Then "First" should appear before "Second"

  Scenario: Header component is replaceable
    Given a default header showing version info
    When a custom header "Custom Title" is set
    Then the rendered header should contain "Custom Title"

  Scenario: Footer component is replaceable
    Given a default footer showing model info
    When a custom footer "Custom Status" is set
    Then the rendered footer should contain "Custom Status"
