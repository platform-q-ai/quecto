@done
Feature: AgentCmdTool — native UDS interaction with spawned subagents
  As an AI agent
  I want the agent_cmd tool to interact with spawned subagents via UDS
  So that I can orchestrate parallel work without external dependencies

  # --- Tool definition ---

  Scenario: Tool definition has correct name
    Given an AgentCmdTool with an empty registry
    Then the agent_cmd tool definition name should be "agent_cmd"
    And the agent_cmd tool definition description should not be empty

  Scenario: Tool definition requires agent_id and command
    Given an AgentCmdTool with an empty registry
    Then the agent_cmd tool definition schema should require "agent_id"
    And the agent_cmd tool definition schema should require "command"

  # --- Argument parsing ---

  Scenario: Parse valid get_state command
    Given an AgentCmdTool with an empty registry
    When I execute agent_cmd with '{"agent_id":"worker-1","command":"get_state"}'
    Then the agent_cmd result should be an error
    And the agent_cmd result should contain "not found"

  Scenario: Parse fails on missing agent_id
    Given an AgentCmdTool with an empty registry
    When I execute agent_cmd with '{"command":"get_state"}'
    Then the agent_cmd result should be an error
    And the agent_cmd result should contain "agent_id"

  Scenario: Parse fails on missing command
    Given an AgentCmdTool with an empty registry
    When I execute agent_cmd with '{"agent_id":"worker-1"}'
    Then the agent_cmd result should be an error
    And the agent_cmd result should contain "command"

  Scenario: Parse fails on invalid JSON
    Given an AgentCmdTool with an empty registry
    When I execute agent_cmd with 'not valid json'
    Then the agent_cmd result should be an error
    And the agent_cmd result should contain "invalid JSON"

  Scenario: Parse fails on unknown command
    Given an AgentCmdTool with an empty registry
    When I execute agent_cmd with '{"agent_id":"worker-1","command":"unknown_cmd"}'
    Then the agent_cmd result should be an error
    And the agent_cmd result should contain "unsupported command"

  # --- Registry lookup ---

  Scenario: Unknown agent_id returns error
    Given an AgentCmdTool with an empty registry
    When I execute agent_cmd with '{"agent_id":"nonexistent","command":"get_state"}'
    Then the agent_cmd result should be an error
    And the agent_cmd result should contain "not found"

  Scenario: Known agent_id is looked up from registry
    Given an AgentCmdTool with a mock registry entry "worker-1"
    When I execute agent_cmd with '{"agent_id":"worker-1","command":"get_state"}'
    Then the agent_cmd result should not be an error

  # --- Command building ---

  Scenario: get_state command is built correctly
    Given an AgentCmdTool with a mock registry entry "w1"
    When I execute agent_cmd with '{"agent_id":"w1","command":"get_state"}'
    Then the agent_cmd should have sent command type "get_state"

  Scenario: get_messages_tail command uses count parameter
    Given an AgentCmdTool with a mock registry entry "w1"
    When I execute agent_cmd with '{"agent_id":"w1","command":"get_messages_tail","count":5}'
    Then the agent_cmd should have sent command type "get_messages_tail"

  Scenario: get_messages_tail defaults count to 1
    Given an AgentCmdTool with a mock registry entry "w1"
    When I execute agent_cmd with '{"agent_id":"w1","command":"get_messages_tail"}'
    Then the agent_cmd should have sent command type "get_messages_tail"

  Scenario: prompt command requires message
    Given an AgentCmdTool with a mock registry entry "w1"
    When I execute agent_cmd with '{"agent_id":"w1","command":"prompt"}'
    Then the agent_cmd result should be an error
    And the agent_cmd result should contain "message"

  Scenario: prompt command sends message
    Given an AgentCmdTool with a mock registry entry "w1"
    When I execute agent_cmd with '{"agent_id":"w1","command":"prompt","message":"Do work"}'
    Then the agent_cmd should have sent command type "prompt"

  Scenario: steer command requires message
    Given an AgentCmdTool with a mock registry entry "w1"
    When I execute agent_cmd with '{"agent_id":"w1","command":"steer"}'
    Then the agent_cmd result should be an error
    And the agent_cmd result should contain "message"

  Scenario: steer command sends message
    Given an AgentCmdTool with a mock registry entry "w1"
    When I execute agent_cmd with '{"agent_id":"w1","command":"steer","message":"Change direction"}'
    Then the agent_cmd should have sent command type "steer"

  Scenario: abort command is built correctly
    Given an AgentCmdTool with a mock registry entry "w1"
    When I execute agent_cmd with '{"agent_id":"w1","command":"abort"}'
    Then the agent_cmd should have sent command type "abort"

  Scenario: get_session_stats command is built correctly
    Given an AgentCmdTool with a mock registry entry "w1"
    When I execute agent_cmd with '{"agent_id":"w1","command":"get_session_stats"}'
    Then the agent_cmd should have sent command type "get_session_stats"

  # --- New commands (#547) ---

  Scenario: follow_up command requires message
    Given an AgentCmdTool with a mock registry entry "w1"
    When I execute agent_cmd with '{"agent_id":"w1","command":"follow_up"}'
    Then the agent_cmd result should be an error
    And the agent_cmd result should contain "message"

  Scenario: follow_up command sends message
    Given an AgentCmdTool with a mock registry entry "w1"
    When I execute agent_cmd with '{"agent_id":"w1","command":"follow_up","message":"After you finish"}'
    Then the agent_cmd should have sent command type "follow_up"

  Scenario: get_messages command is built correctly
    Given an AgentCmdTool with a mock registry entry "w1"
    When I execute agent_cmd with '{"agent_id":"w1","command":"get_messages"}'
    Then the agent_cmd should have sent command type "get_messages"

  Scenario: set_model command requires model parameter
    Given an AgentCmdTool with a mock registry entry "w1"
    When I execute agent_cmd with '{"agent_id":"w1","command":"set_model"}'
    Then the agent_cmd result should be an error
    And the agent_cmd result should contain "model"

  Scenario: set_model command sends model
    Given an AgentCmdTool with a mock registry entry "w1"
    When I execute agent_cmd with '{"agent_id":"w1","command":"set_model","model":"anthropic/claude-sonnet-4-20250514"}'
    Then the agent_cmd should have sent command type "set_model"

  Scenario: clear_history command is built correctly
    Given an AgentCmdTool with a mock registry entry "w1"
    When I execute agent_cmd with '{"agent_id":"w1","command":"clear_history"}'
    Then the agent_cmd should have sent command type "clear_history"

  Scenario: get_subagents command is built correctly
    Given an AgentCmdTool with a mock registry entry "w1"
    When I execute agent_cmd with '{"agent_id":"w1","command":"get_subagents"}'
    Then the agent_cmd should have sent command type "get_subagents"

  Scenario: get_extensions command is built correctly
    Given an AgentCmdTool with a mock registry entry "w1"
    When I execute agent_cmd with '{"agent_id":"w1","command":"get_extensions"}'
    Then the agent_cmd should have sent command type "get_extensions"

  Scenario: reload_extensions command is built correctly
    Given an AgentCmdTool with a mock registry entry "w1"
    When I execute agent_cmd with '{"agent_id":"w1","command":"reload_extensions"}'
    Then the agent_cmd should have sent command type "reload_extensions"

  # --- UDS transport (#557) ---

  Scenario: UDS connection keeps write half open until response received
    Given a live UDS subagent
    When I send get_state via agent_cmd
    Then the response should contain "isStreaming"
    And the response should be valid JSON with type "response"

  Scenario: get_messages_tail returns conversation history
    Given a live UDS subagent with conversation history
    When I send get_messages_tail with count 2 via agent_cmd
    Then the response should contain message data
