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

  # A genuinely DIFFERENT command (get_session_stats) must never be answered by
  # the connect-time get_messages snapshot — the #835 id-correlation guarantee.
  Scenario: mismatched command against a busy child preserves id-correlation
    Given an AgentCmdTool with a busy mock registry entry "busy-skip"
    When I execute agent_cmd with '{"agent_id":"busy-skip","command":"get_session_stats"}'
    Then the agent_cmd result should not be an error
    And the agent_cmd result should contain "LATEST TURNS"
    And the agent_cmd result should not contain "FIRST MESSAGE ONLY"

  # --- Counted / tail get_messages served from the snapshot (#842) ---

  # A counted get_messages against a busy child must return the last-N snapshot
  # without blocking to the 300s deadline (the mock holds the connection open and
  # never sends an id-matched reply, so a snapshot-acceptance regression would
  # hang to that deadline rather than fail fast — completion proves acceptance).
  Scenario: counted get_messages against a busy child accepts the snapshot tail
    Given an AgentCmdTool with a busy multi-message snapshot registry entry "busy-tail"
    When I execute agent_cmd with '{"agent_id":"busy-tail","command":"get_messages","count":1}'
    Then the agent_cmd result should not be an error
    And the agent_cmd response command "get_messages" should include a "messages" array
    And the agent_cmd response command "get_messages" should include boolean field "snapshot" set to "true"
    And the agent_cmd result should contain "NEWEST MESSAGE"
    And the agent_cmd result should not contain "OLDEST MESSAGE"

  Scenario: get_messages_tail alias against a busy child accepts the snapshot tail
    Given an AgentCmdTool with a busy multi-message snapshot registry entry "busy-tail2"
    When I execute agent_cmd with '{"agent_id":"busy-tail2","command":"get_messages_tail","count":1}'
    Then the agent_cmd result should not be an error
    And the agent_cmd response command "get_messages" should include a "messages" array
    And the agent_cmd response command "get_messages" should include boolean field "snapshot" set to "true"
    And the agent_cmd result should contain "NEWEST MESSAGE"
    And the agent_cmd result should not contain "OLDEST MESSAGE"

  # --- get_subagents served on the busy path (#874) ---

  # Acceptance criteria for #874:
  # - get_subagents against a BUSY child returns the child's current registry
  #   view within the inspector timeout instead of queuing behind the child's
  #   turn (the master's own agent_cmd get_subagents call must not hang).
  # - The snapshot is tagged snapshot:true so a caller can tell the data may lag
  #   the in-flight turn (#842 consistency).
  # - id-correlation (#835) is preserved: a DIFFERENT command against a busy
  #   child that pushes a get_subagents snapshot must NOT be answered by that
  #   snapshot (no regression to the "first message only" class).
  # - Idle behaviour remains unchanged (the dispatch loop answers in FIFO order).

  # The mock busy child pushes a get_subagents snapshot on connect and NEVER
  # sends an id-matched reply, so a blocking-on-reply regression would ride the
  # ~300s deadline instead of completing — completing proves the snapshot was
  # accepted and served promptly.
  Scenario: get_subagents against a busy child accepts the connect-time snapshot
    Given an AgentCmdTool with a busy subagents snapshot registry entry "busy-subagents"
    When I execute agent_cmd with '{"agent_id":"busy-subagents","command":"get_subagents"}'
    Then the agent_cmd result should not be an error
    And the agent_cmd response command "get_subagents" should include a "subagents" array
    And the agent_cmd response command "get_subagents" should include boolean field "snapshot" set to "true"
    And the agent_cmd result should contain "grandchild-worker"

  # A genuinely DIFFERENT command (get_session_stats) must never be answered by
  # the connect-time get_subagents snapshot — the #835 id-correlation guarantee.
  # This mock pushes a get_subagents snapshot AND echoes the real command with an
  # id-matched reply: get_session_stats must SKIP the snapshot and accept only
  # its own correlated reply.
  Scenario: mismatched command against a busy subagents-snapshot child preserves id-correlation
    Given an AgentCmdTool with a busy subagents snapshot and echo registry entry "busy-subagents-skip"
    When I execute agent_cmd with '{"agent_id":"busy-subagents-skip","command":"get_session_stats"}'
    Then the agent_cmd result should not be an error
    And the agent_cmd result should contain "LATEST TURNS"
    And the agent_cmd result should not contain "grandchild-worker"

  # --- Non-blocking control forwards (#876) ---

  # Acceptance criteria for #876:
  # - prompt/steer/follow_up/abort against a BUSY child return on the child's
  #   ACCEPTANCE ack within the inspector timeout (never the 300s turn deadline),
  #   so the parent's turn is not frozen for the child's turn duration.
  # - id-correlation (#835) is preserved: the acceptance ack echoes the request id.
  # - Completion still surfaces later via the auto-await note / await (unchanged).
  # The mock child acks acceptance but NEVER sends a turn-completion response and
  # holds the connection open, so a blocking-on-completion regression would hang
  # to the 300s deadline rather than fail fast — completion proves acceptance.

  Scenario: prompt against a busy child returns on acceptance
    Given an AgentCmdTool with a fast-ack busy registry entry "busy-prompt876"
    When I execute agent_cmd with '{"agent_id":"busy-prompt876","command":"prompt","message":"do work"}'
    Then the agent_cmd result should not be an error
    And the agent_cmd result should contain "success"

  Scenario: follow_up against a busy child returns on acceptance
    Given an AgentCmdTool with a fast-ack busy registry entry "busy-fu876"
    When I execute agent_cmd with '{"agent_id":"busy-fu876","command":"follow_up","message":"after"}'
    Then the agent_cmd result should not be an error
    And the agent_cmd result should contain "success"

  Scenario: steer against a busy child returns on acceptance
    Given an AgentCmdTool with a fast-ack busy registry entry "busy-steer876"
    When I execute agent_cmd with '{"agent_id":"busy-steer876","command":"steer","message":"turn"}'
    Then the agent_cmd result should not be an error
    And the agent_cmd result should contain "success"

  Scenario: abort against a busy child returns on acceptance
    Given an AgentCmdTool with a fast-ack busy registry entry "busy-abort876"
    When I execute agent_cmd with '{"agent_id":"busy-abort876","command":"abort"}'
    Then the agent_cmd result should not be an error
    And the agent_cmd result should contain "success"

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
