@tui @pending
Feature: TUI subagent activity rendering — match Pi TUI style
  Issue #472: spawn and agent_cmd tools render identically to any other tool.
  They should have visually distinct rendering with subagent status indicators,
  nested indentation, and different border styling.

  # ---------------------------------------------------------------------------
  # Subagent tool detection
  # ---------------------------------------------------------------------------

  Scenario: spawn tool detected as subagent tool
    Given a chat component
    When a tool_start event arrives with tool_name "spawn"
    Then the chat entry should be marked as a subagent tool

  Scenario: agent_cmd tool detected as subagent tool
    Given a chat component
    When a tool_start event arrives with tool_name "agent_cmd"
    Then the chat entry should be marked as a subagent tool

  Scenario: regular tool not detected as subagent tool
    Given a chat component
    When a tool_start event arrives with tool_name "bash"
    Then the chat entry should NOT be marked as a subagent tool

  # ---------------------------------------------------------------------------
  # Subagent spawn rendering
  # ---------------------------------------------------------------------------

  Scenario: spawn tool shows agent label from args
    Given a chat component
    When a spawn tool_start event arrives with args {"agent":"reviewer","task":"Review PR"}
    Then the rendered output should contain "reviewer"
    And the rendered output should contain a subagent icon

  Scenario: spawn tool completion shows success with duration
    Given a chat component
    And a spawn tool is running for agent "reviewer"
    When the spawn tool completes successfully in 1500ms
    Then the rendered output should show a success icon
    And the rendered output should contain "1500ms"

  # ---------------------------------------------------------------------------
  # Subagent agent_cmd rendering
  # ---------------------------------------------------------------------------

  Scenario: agent_cmd steer shows action and target
    Given a chat component
    When an agent_cmd tool_start event arrives with args {"action":"steer","agentId":"reviewer","message":"focus on tests"}
    Then the rendered output should contain "steer"
    And the rendered output should contain "reviewer"

  Scenario: agent_cmd follow_up shows action and target
    Given a chat component
    When an agent_cmd tool_start event arrives with args {"action":"follow_up","agentId":"builder","message":"also fix lint"}
    Then the rendered output should contain "follow_up"
    And the rendered output should contain "builder"

  Scenario: agent_cmd get_state shows agent status query
    Given a chat component
    When an agent_cmd tool_start event arrives with args {"action":"get_state","agentId":"reviewer"}
    Then the rendered output should contain "get_state"
    And the rendered output should contain "reviewer"

  Scenario: agent_cmd abort shows abort action
    Given a chat component
    When an agent_cmd tool_start event arrives with args {"action":"abort","agentId":"builder"}
    Then the rendered output should contain "abort"
    And the rendered output should contain "builder"

  # ---------------------------------------------------------------------------
  # Visual distinction
  # ---------------------------------------------------------------------------

  Scenario: subagent tools use distinct prefix icon
    Given a chat component
    When a spawn tool_start event arrives
    Then the rendered line should use a subagent prefix icon different from regular tools

  Scenario: subagent tool result uses nested indentation
    Given a chat component
    And a completed agent_cmd tool with result "Agent is idle"
    Then the result preview should have deeper indentation than regular tool results

  # ---------------------------------------------------------------------------
  # Width compliance
  # ---------------------------------------------------------------------------

  Scenario: subagent tool lines respect terminal width
    Given a chat component with subagent tool entries
    When rendered at width 40
    Then no rendered line should exceed 40 visible characters
