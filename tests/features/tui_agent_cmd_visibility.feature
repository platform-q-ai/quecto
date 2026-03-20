@tui
Feature: TUI shows agent_cmd query responses in the chat (#538)
  As a TUI user
  I want to see agent_cmd query results in the chat output
  So that I can inspect subagent state, messages, and stats

  Scenario: agent_cmd query tool output is shown in chat
    Given the agent calls agent_cmd with command "get_state"
    When the tool execution completes with a JSON response
    Then the tool result should be displayed in the chat area
    And the tool output box should show the response content

  Scenario: spawn tool output is suppressed in chat
    Given the agent calls spawn with agent_id "worker-1"
    When the tool execution completes
    Then the tool result should NOT be displayed in the chat area
    And the subagent status bar should show the agent instead

  Scenario: agent_cmd result visibility decision
    Given a tool_name "agent_cmd"
    Then the tool result should be visible
    Given a tool_name "spawn"
    Then the tool result should be suppressed
