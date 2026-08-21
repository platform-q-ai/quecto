@done
Feature: Persistent subagent monitor — live event stream from child agents
  As a parent agent
  I want automatic real-time monitoring of spawned child agents
  So that I can track their status without polling

  # --- SubagentStatus enum ---

  Scenario: SubagentStatus defaults to Starting
    Given a new SubagentEntry with status Starting
    Then the subagent status should be "Starting"

  Scenario: SubagentStatus displays correctly for all variants
    Given subagent status variants Starting, Idle, Running, Error, Exited
    Then each variant should have a distinct display string

  # --- State transition logic ---

  Scenario: agent_start event transitions status to Running
    Given a SubagentEntry with status Idle
    When the monitor receives an "agent_start" event
    Then the subagent status should be "Running"

  Scenario: agent_end event transitions status to Idle
    Given a SubagentEntry with status Running
    When the monitor receives an "agent_end" event
    Then the subagent status should be "Idle"

  Scenario: tool_execution_start updates status and last_tool
    Given a SubagentEntry with status Running
    When the monitor receives a "tool_execution_start" event with tool_name "bash"
    Then the subagent status should be "Running"
    And the last_tool should be "bash"

  Scenario: tool_execution_end with is_error true stays Running
    Given a SubagentEntry with status Running
    When the monitor receives a "tool_execution_end" event with is_error true and tool_name "bash"
    Then the subagent status should be "Running"
    And the last_error should be None

  Scenario: tool_execution_end with is_error false keeps Running
    Given a SubagentEntry with status Running
    When the monitor receives a "tool_execution_end" event with is_error false and tool_name "bash"
    Then the subagent status should be "Running"

  Scenario: connection closed transitions to Exited
    Given a SubagentEntry with status Running
    When the monitor detects connection closed
    Then the subagent status should be "Exited"

  # --- Extended SubagentEntry ---

  Scenario: SubagentEntry tracks socket_path and pid alongside status
    Given a SubagentEntry with socket_path "/tmp/test.sock" and pid 1234
    Then the subagent entry should have socket_path "/tmp/test.sock"
    And the subagent entry should have pid 1234
    And the subagent status should be "Starting"

  Scenario: SubagentEntry last_tool starts as None
    Given a new SubagentEntry with status Starting
    Then the last_tool should be None

  Scenario: SubagentEntry last_error starts as None
    Given a new SubagentEntry with status Starting
    Then the last_error should be None

  # --- apply_event pure function ---

  Scenario: apply_event handles agent_start JSON
    Given a SubagentEntry with status Idle
    When apply_event is called with '{"type":"agent_start"}'
    Then the subagent status should be "Running"

  Scenario: apply_event handles agent_end JSON
    Given a SubagentEntry with status Running
    When apply_event is called with '{"type":"agent_end","messages":[]}'
    Then the subagent status should be "Idle"

  Scenario: apply_event handles tool_execution_start JSON
    Given a SubagentEntry with status Running
    When apply_event is called with '{"type":"tool_execution_start","toolCallId":"c1","toolName":"grep","args":{}}'
    Then the subagent status should be "Running"
    And the last_tool should be "grep"

  Scenario: apply_event handles tool_execution_end with error JSON as child-local
    Given a SubagentEntry with status Running
    When apply_event is called with '{"type":"tool_execution_end","toolCallId":"c1","toolName":"edit","result":{"content":[]},"isError":true}'
    Then the subagent status should be "Running"
    And the last_error should be None

  Scenario: apply_event handles tool_execution_end without error JSON
    Given a SubagentEntry with status Running
    When apply_event is called with '{"type":"tool_execution_end","toolCallId":"c1","toolName":"read","result":{"content":[]},"isError":false}'
    Then the subagent status should be "Running"

  Scenario: apply_event ignores unknown event types
    Given a SubagentEntry with status Idle
    When apply_event is called with '{"type":"token","token":"hello"}'
    Then the subagent status should be "Idle"

  Scenario: apply_event ignores malformed JSON
    Given a SubagentEntry with status Idle
    When apply_event is called with 'not valid json'
    Then the subagent status should be "Idle"

  # --- Registry integration ---

  Scenario: Spawn registers child with Starting status
    Given a SpawnTool with empty allowlist
    When I execute the SpawnTool with '{"task":"work","agent_id":"monitor-test"}'
    Then the subagent registry should contain "monitor-test"
    And the subagent registry entry "monitor-test" should have status "Starting"

  # --- Grandchild propagation (#815) ---

  Scenario: a child's subagent_state_changed is forwarded preserving every descendant's identity
    Given a child's subagent_state_changed listing grandchild "gc-a" under "child-1" and grandchild "gc-b" under "child-2"
    When the monitor forwards the child's subagent_state_changed event
    Then the forwarded event should list "gc-a" with parent_id "child-1"
    And the forwarded event should list "gc-b" with parent_id "child-2"

  # --- Monitor task abort handle ---

  Scenario: Monitor task can be aborted via JoinHandle
    Given a monitor abort handle
    When the abort handle is triggered
    Then the monitor task should be cancelled

  # --- Cascade-remove + broadcast on exit/kill (#831) ---

  Scenario: killing a parent cascade-removes its whole subtree and broadcasts survivors
    Given a root registry with parent "p", child "c" under "p", and grandchild "gc" under "c", plus a live agent "live"
    When the parent "p" is killed
    Then the broadcast subagent_state_changed should list only "live"
    And the registry should no longer contain "p", "c", or "gc"
    And the registry should still contain "live"

  Scenario: a forwarded push prunes a grandchild the child no longer reports
    Given a root registry with child "child" and a previously-merged grandchild "gc" under it
    When the child "child" forwards a subagent_state_changed with no descendants
    Then the forwarded event should not list "gc"
    And the registry should contain dead tombstone "gc"
    And the registry should still contain "child"

  Scenario: a descendant push preserves root siblings while adding grandchildren
    Given a root registry with child "child" and sibling "sibling"
    When child "child" forwards grandchild "gc" as running
    Then the forwarded event should list "child"
    And the forwarded event should list "sibling"
    And the forwarded event should list "gc" with parent_id "child"

  Scenario: a child cannot merge more than the forwarded descendant cap
    Given an empty root subagent registry
    When child "child" forwards 300 running grandchildren
    Then the registry should contain 256 subagents
    And the forwarded event should contain 256 subagents

  # --- Prompt idle propagation for nested agents (#839) ---

  Scenario: a nested idle agent stays idle when a descendant update lacks status
    Given a root monitor knows grandchild "gc" under "child" is idle
    When the child reports grandchild "gc" without a status update
    Then the monitor should keep "gc" idle
    And observers should see "gc" as idle

  Scenario: child completion immediately updates observers to idle
    Given a root monitor with running child "child"
    When the child "child" finishes its turn
    Then observers should receive subagent state listing "child" as idle

  Scenario: idle nested grandchild is not reported as working in the subagent panel
    Given a forwarded subagent state listing idle grandchild "gc" under "child"
    When the TUI renders the forwarded subagent state
    Then the subagent panel should show "gc" as idle
    And the subagent panel should not count "gc" as working

  # --- Prompt running visibility on a long first turn (#866) ---

  Scenario: a child's first running transition is broadcast to observers
    Given a root monitor with running child "child"
    When the child "child" starts its turn
    Then observers should receive subagent state listing "child" as running

  Scenario: an unconfirmed spawning agent stays visible when a push omits it
    Given the TUI is tracking a spawning agent "newbie"
    When a subagent push arrives that omits "newbie"
    Then the subagent panel should still show "newbie"

  Scenario: a removal request for an unknown agent emits no broadcast
    Given a root registry with parent "p", child "c" under "p", and grandchild "gc" under "c", plus a live agent "live"
    When an unknown agent "ghost" is reported gone
    Then no subagent_state_changed broadcast is emitted
    And the registry should still contain "live"
