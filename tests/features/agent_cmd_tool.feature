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

  Scenario: get_messages command uses count parameter for tail reads
    Given an AgentCmdTool with a mock registry entry "w1"
    When I execute agent_cmd with '{"agent_id":"w1","command":"get_messages","count":5}'
    Then the agent_cmd should have sent command type "get_messages"
    And the agent_cmd should have sent count 5

  Scenario: deprecated get_messages_tail aliases to get_messages count
    Given an AgentCmdTool with a mock registry entry "w1"
    When I execute agent_cmd with '{"agent_id":"w1","command":"get_messages_tail","count":5}'
    Then the agent_cmd should have sent command type "get_messages"
    And the agent_cmd should have sent count 5

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

  Scenario: tool description presents one conversation inspection command
    Given an AgentCmdTool with an empty registry
    Then the agent_cmd tool definition description should contain "get_messages"
    And the agent_cmd tool definition description should contain "count"
    And the agent_cmd tool definition description should not contain "get_messages_tail"

  Scenario: set_model command requires model parameter
    Given an AgentCmdTool with a mock registry entry "w1"
    When I execute agent_cmd with '{"agent_id":"w1","command":"set_model"}'
    Then the agent_cmd result should be an error
    And the agent_cmd result should contain "model"

  Scenario: set_model command sends model
    Given an AgentCmdTool with a mock registry entry "w1"
    When I execute agent_cmd with '{"agent_id":"w1","command":"set_model","model":"anthropic/claude-sonnet-4-6"}'
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

  # --- Kill command (#559) ---

  Scenario: kill command is built correctly
    Given an AgentCmdTool with a mock registry entry "w1"
    When I execute agent_cmd with '{"agent_id":"w1","command":"kill"}'
    Then the agent_cmd result should not be an error
    And the agent_cmd result should contain "killed"

  Scenario: kill unknown agent returns error
    Given an AgentCmdTool with an empty registry
    When I execute agent_cmd with '{"agent_id":"nonexistent","command":"kill"}'
    Then the agent_cmd result should be an error
    And the agent_cmd result should contain "not found"

  # --- UDS transport (#557) ---
  # Verified via unit tests in agent_cmd.rs (mock UDS server).

  # --- Busy-child snapshots (#837) ---

  # Acceptance criteria for #837:
  # - get_messages and get_state against a busy child return a useful snapshot within the inspector timeout.
  # - Snapshot responses are correct-shaped and reflect at least the child's last completed turn/state.
  # - id-correlation is still required for commands where a connect-time snapshot is not a valid answer.
  # - get_messages_tail is folded into get_messages count in the task-facing tool surface.
  # - Idle behaviour remains unchanged while consolidated parsing is covered.

  Scenario: get_messages against a busy child accepts the connect-time snapshot
    Given an AgentCmdTool with a busy snapshot registry entry "busy-snapshot"
    When I execute agent_cmd with '{"agent_id":"busy-snapshot","command":"get_messages"}'
    Then the agent_cmd result should not be an error
    And the agent_cmd response command "get_messages" should include a "messages" array
    And the agent_cmd result should contain "FIRST MESSAGE ONLY"

  Scenario: get_state against a busy child returns a status snapshot
    Given an AgentCmdTool with a busy state snapshot registry entry "busy-state"
    When I execute agent_cmd with '{"agent_id":"busy-state","command":"get_state"}'
    Then the agent_cmd result should not be an error
    And the agent_cmd response command "get_state" should include boolean field "isStreaming"
    And the agent_cmd response command "get_state" should include integer field "messageCount"

  Scenario: non-snapshot command against a busy child preserves id-correlation
    Given an AgentCmdTool with a busy mock registry entry "busy-skip"
    When I execute agent_cmd with '{"agent_id":"busy-skip","command":"get_messages","count":1}'
    Then the agent_cmd result should not be an error
    And the agent_cmd result should contain "LATEST TURNS"
    And the agent_cmd result should not contain "FIRST MESSAGE ONLY"

  @pending
  Scenario: UDS connection keeps write half open until response received
    Given a live UDS subagent
    When I send get_state via agent_cmd
    Then the response should contain "isStreaming"
    And the response should be valid JSON with type "response"

  @pending
  Scenario: get_messages with count returns conversation tail
    Given a live UDS subagent with conversation history
    When I send get_messages with count 2 via agent_cmd
    Then the response should contain [message] data
