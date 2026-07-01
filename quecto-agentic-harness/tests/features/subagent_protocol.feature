Feature: Subagent protocol commands (#524)
  As an external UDS client (TUI, script, test harness)
  I want to query subagent state and receive real-time state change events
  So that I can display live child agent status

  # ─── get_subagents command ──────────────────────────────────────────────────

  @wip
  Scenario: get_subagents returns empty list when no subagents exist
    Given a SubagentInfo list from an empty registry
    Then the subagent info list should be empty

  @wip
  Scenario: get_subagents identifies read-only observers
    Given a read-only observer sub-agent "reviewer" is registered
    And a read-write sub-agent "formatter" is registered
    When a client requests sub-agent state
    Then the subagent info list should have 2 entries
    And subagent info "reviewer" should have status "running"
    And subagent info "reviewer" should have last_tool "bash"
    And subagent info "reviewer" should have pid 1234
    And subagent info "reviewer" should be read-only
    And subagent info "formatter" should have status "idle"
    And subagent info "formatter" should be read-write
    And the subagent info list should contain both observer and read-write states

  @wip
  Scenario: get_subagents includes error status and last_error
    Given a registry with subagent "linter" status "error" last_tool "bash" pid 9999
    And subagent "linter" has last_error "tool 'bash' returned error"
    When I build a SubagentInfo list from the registry
    Then subagent info "linter" should have status "error"
    And subagent info "linter" should have last_error "tool 'bash' returned error"

  @wip
  Scenario: get_subagents includes exited subagents
    Given a registry with subagent "worker" status "exited" last_tool "" pid 0
    When I build a SubagentInfo list from the registry
    Then subagent info "worker" should have status "exited"

  @wip
  Scenario: get_subagents includes starting subagents
    Given a registry with subagent "init" status "starting" last_tool "" pid 42
    When I build a SubagentInfo list from the registry
    Then subagent info "init" should have status "starting"

  # ─── SubagentInfo serialization ──────────────────────────────────────────────

  @wip
  Scenario: SubagentInfo serializes to camelCase JSON
    Given a SubagentInfo with agentId "test" status "running" lastTool "bash" pid 123
    When I serialize the SubagentInfo to JSON
    Then the JSON should contain key "agentId" with value "test"
    And the JSON should contain key "status" with value "running"
    And the JSON should contain key "lastTool" with value "bash"
    And the JSON should contain key "pid" with value 123

  @wip
  Scenario: SubagentInfo serializes null lastTool when absent
    Given a SubagentInfo with agentId "idle-agent" status "idle" lastTool "" pid 456
    When I serialize the SubagentInfo to JSON
    Then the JSON should contain key "lastTool" with null value

  @wip
  Scenario: SubagentInfo serializes lastError when present
    Given a SubagentInfo with agentId "err" status "error" lastTool "" pid 0
    And the SubagentInfo has lastError "connection refused"
    When I serialize the SubagentInfo to JSON
    Then the JSON should contain key "lastError" with value "connection refused"

  # ─── get_subagents command parsing ──────────────────────────────────────────

  @wip
  Scenario: get_subagents command parses from JSON
    Given the JSON command '{"type":"get_subagents","id":"gs-1"}'
    When I parse the command
    Then the command type should be "get_subagents"
    And the command id should be "gs-1"

  @wip
  Scenario: get_subagents command parses without id
    Given the JSON command '{"type":"get_subagents"}'
    When I parse the command
    Then the command type should be "get_subagents"
    And the command id should be absent

  # ─── subagent_state_changed event ──────────────────────────────────────────

  @wip
  Scenario: subagent_state_changed event serializes correctly
    Given a SubagentStateChanged event with 2 subagents
    When I serialize the event to JSON
    Then the JSON should contain "type" with value "subagent_state_changed"
    And the JSON should contain a "subagents" array with 2 entries

  @wip
  Scenario: subagent_state_changed event round-trips through serde
    Given a SubagentStateChanged event with 1 subagent "monitor-test" status "running"
    When I serialize and deserialize the event
    Then the deserialized event should be SubagentStateChanged
    And the deserialized subagents should contain "monitor-test" with status "running"

  @wip
  Scenario: subagent_state_changed event preserves observer status
    Given a SubagentStateChanged event for read-only sub-agent "reviewer" and read-write sub-agent "worker"
    When I serialize and deserialize the event
    Then the deserialized event should be SubagentStateChanged
    And the deserialized subagents should contain "reviewer" as read-only
    And the deserialized subagents should contain "worker" as read-write

  # ─── build_subagent_info_list helper ─────────────────────────────────────────

  @wip
  Scenario: build_subagent_info_list sorts by agent_id
    Given a registry with subagent "zebra" status "idle" last_tool "" pid 1
    And a registry with subagent "alpha" status "running" last_tool "bash" pid 2
    When I build a SubagentInfo list from the registry
    Then the first subagent info should have agentId "alpha"
    And the second subagent info should have agentId "zebra"

  # ─── socket_path exposure for connect-on-select (#800) ───────────────────────
  # The TUI lazily opens a direct UDS connection to a SELECTED sub-agent's own
  # socket. To do that it must learn the socket path; the kernel surfaces it on
  # each SubagentInfo (local use only). The registry already knows the path.

  @wip
  Scenario: build_subagent_info_list surfaces each subagent's socket_path
    Given a registry with subagent "worker" status "running" last_tool "bash" pid 7
    When I build a SubagentInfo list from the registry
    Then subagent info "worker" should have socketPath "/tmp/test.sock"

  @wip
  Scenario: a SubagentInfo round-trips its socketPath over the wire
    Given a registry with subagent "worker" status "running" last_tool "bash" pid 7
    When I build a SubagentInfo list from the registry
    And I serialize the first subagent info
    Then the round-tripped subagent info should have socketPath "/tmp/test.sock"

  @wip
  Scenario: build_subagent_info_list maps all status values
    Given a registry with subagent "a" status "starting" last_tool "" pid 1
    And a registry with subagent "b" status "idle" last_tool "" pid 2
    And a registry with subagent "c" status "running" last_tool "read" pid 3
    And a registry with subagent "d" status "error" last_tool "" pid 4
    And a registry with subagent "e" status "exited" last_tool "" pid 5
    When I build a SubagentInfo list from the registry
    Then subagent info "a" should have status "starting"
    And subagent info "b" should have status "idle"
    And subagent info "c" should have status "running"
    And subagent info "d" should have status "error"
    And subagent info "e" should have status "exited"
