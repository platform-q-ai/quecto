Feature: Subagent status bar widget (#525)
  As a human operator using the TUI
  I want to see live subagent status in compact progress bars
  So that I can monitor child agent activity without polling

  # These scenarios verify the SubagentBar component rendering logic.
  # The actual component tests are in quecto-tui/src/interface/components/subagent_bar.rs.

  @wip
  Scenario: SubagentBar renders nothing when empty
    Given a SubagentBar with no agents
    When I render the bar at width 80
    Then the rendered output should be empty

  @wip
  Scenario: SubagentBar renders one line per agent
    Given a SubagentBar with agents:
      | agent_id  | status  | last_tool | last_error |
      | reviewer  | running | bash      |            |
      | formatter | idle    |           |            |
    When I render the bar at width 80
    Then the rendered output should have 2 lines
    And the first line should contain "reviewer"
    And the first line should contain "Running"
    And the second line should contain "formatter"
    And the second line should contain "Idle"

  @wip
  Scenario: SubagentBar shows error context
    Given a SubagentBar with agents:
      | agent_id | status | last_tool | last_error           |
      | linter   | error  |           | tool 'bash' returned |
    When I render the bar at width 80
    Then the first line should contain "Error"
    And the first line should contain "tool 'bash' returned"

  @wip
  Scenario: SubagentBar shows running tool context
    Given a SubagentBar with agents:
      | agent_id | status  | last_tool | last_error |
      | worker   | running | read      |            |
    When I render the bar at width 80
    Then the first line should contain "Running"
    And the first line should contain "read"

  @wip
  Scenario: SubagentBar hides when all agents cleared
    Given a SubagentBar with agents:
      | agent_id | status | last_tool | last_error |
      | worker   | idle   |           |            |
    When I update the bar with an empty list
    And I render the bar at width 80
    Then the rendered output should be empty
