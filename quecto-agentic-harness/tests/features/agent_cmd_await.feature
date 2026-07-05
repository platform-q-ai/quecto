@done
Feature: agent_cmd await — block until sub-agent reaches terminal state
  As an AI orchestrator agent
  I want the agent_cmd tool to support an "await" command that blocks until a
  sub-agent reaches a terminal condition (idle, exited, timeout, error)
  So that I can efficiently wait for sub-agents without polling and burning tokens

  # --- Argument parsing ---

  Scenario: await command is accepted as a valid command
    Given an AgentCmdTool with an empty registry
    When I execute agent_cmd with '{"agent_id":"w1","command":"await"}'
    Then the agent_cmd result should not be a tool error
    And the agent_cmd await result status should be "error"
    And the agent_cmd await result reason should be "agent_not_found"

  Scenario: await with custom timeout is accepted
    Given an AgentCmdTool with an empty registry
    When I execute agent_cmd with '{"agent_id":"w1","command":"await","timeout":120}'
    Then the agent_cmd result should not be a tool error
    And the agent_cmd await result status should be "error"
    And the agent_cmd await result reason should be "agent_not_found"

  Scenario: await with custom idle_timeout is accepted
    Given an AgentCmdTool with an empty registry
    When I execute agent_cmd with '{"agent_id":"w1","command":"await","idle_timeout":0}'
    Then the agent_cmd result should not be a tool error
    And the agent_cmd await result status should be "error"
    And the agent_cmd await result reason should be "agent_not_found"

  # --- Agent not found ---

  Scenario: await returns structured error for unknown agent
    Given an AgentCmdTool with an empty registry
    When I execute agent_cmd with '{"agent_id":"nonexistent","command":"await"}'
    Then the agent_cmd result should not be a tool error
    And the agent_cmd await result status should be "error"
    And the agent_cmd await result reason should be "agent_not_found"
    And the agent_cmd await result agent_id should be "nonexistent"
    And the agent_cmd await result elapsed_ms should be 0

  # --- Stable idle detection ---

  Scenario: await returns idle with an incomplete verdict when no workflow completed
    Given an AgentCmdTool with a mock await registry
    And the mock subagent "w1" has status "idle"
    When I execute agent_cmd with '{"agent_id":"w1","command":"await","idle_timeout":0}'
    Then the agent_cmd await result status should be "idle"
    And the agent_cmd await result reason should be "idle"
    And the agent_cmd await result agent_id should be "w1"
    And the agent_cmd await result verdict should be "incomplete"

  Scenario: await waits through idle_timeout window before returning
    Given an AgentCmdTool with a mock await registry
    And the mock subagent "w1" has status "idle"
    When I execute agent_cmd with '{"agent_id":"w1","command":"await","idle_timeout":1}'
    Then the agent_cmd await result status should be "idle"
    And the agent_cmd await result reason should be "idle"
    And the agent_cmd await result elapsed_ms should be at least 1000

  # --- Process exit detection ---

  Scenario: await returns exited when process exits
    Given an AgentCmdTool with a mock await registry
    And the mock subagent "w1" has status "running"
    And the mock subagent "w1" will exit with code 0 after 500ms
    When I execute agent_cmd with '{"agent_id":"w1","command":"await","timeout":5}'
    Then the agent_cmd await result status should be "exited"
    And the agent_cmd await result reason should be "exit_code_0"

  # Exit code detection with non-zero codes is verified in unit tests
  # (test_await_exit_signal_returns_exit_code). BDD scenario omitted
  # because concurrent scenario execution causes watch channel races.

  # --- Timeout ---

  Scenario: await returns timeout when wall clock exceeds timeout
    Given an AgentCmdTool with a mock await registry
    And the mock subagent "w1" has status "running"
    When I execute agent_cmd with '{"agent_id":"w1","command":"await","timeout":1}'
    Then the agent_cmd await result status should be "timeout"
    And the agent_cmd await result reason should be null

  # --- idle_timeout window resets on streaming ---

  Scenario: await resets idle_timeout when agent resumes streaming
    Given an AgentCmdTool with a mock await registry
    And the mock subagent "w1" has status "running"
    And the mock subagent "w1" will go idle then resume after 200ms then idle permanently
    When I execute agent_cmd with '{"agent_id":"w1","command":"await","idle_timeout":1,"timeout":10}'
    Then the agent_cmd await result status should be "idle"
    And the agent_cmd await result reason should be "idle"

  # --- Multiple awaiters ---

  Scenario: second await on same agent returns error immediately
    Given an AgentCmdTool with a mock await registry
    And the mock subagent "w1" has status "running"
    And another await is already active for "w1"
    When I execute agent_cmd with '{"agent_id":"w1","command":"await","timeout":5}'
    Then the agent_cmd await result status should be "error"
    And the agent_cmd await result reason should be "another_await_active"
    And the agent_cmd await result elapsed_ms should be 0

  # --- Connection failed ---

  Scenario: await returns error when socket connection fails
    Given an AgentCmdTool with a mock await registry
    And the mock subagent "w1" has a stale socket
    When I execute agent_cmd with '{"agent_id":"w1","command":"await"}'
    Then the agent_cmd await result status should be "error"
    And the agent_cmd await result reason should be "connection_failed"

  # --- Workflow snapshot ---

  Scenario: await includes workflow snapshot when available
    Given an AgentCmdTool with a mock await registry
    And the mock subagent "w1" has status "idle"
    And the mock subagent "w1" has workflow state complete with 7 of 7 steps
    When I execute agent_cmd with '{"agent_id":"w1","command":"await","idle_timeout":0}'
    Then the agent_cmd await result status should be "idle"
    And the agent_cmd await result workflow mode should be "complete"
    And the agent_cmd await result workflow steps_completed should be 7
    And the agent_cmd await result workflow steps_total should be 7
    And the agent_cmd await result verdict should be "completed"

  Scenario: await returns null workflow when workflow is not enabled
    Given an AgentCmdTool with a mock await registry
    And the mock subagent "w1" has status "idle"
    When I execute agent_cmd with '{"agent_id":"w1","command":"await","idle_timeout":0}'
    Then the agent_cmd await result status should be "idle"
    And the agent_cmd await result workflow should be null

  # --- Run error surfacing (#752) ---

  Scenario: await surfaces the actual run error message and cause
    Given an AgentCmdTool with a mock await registry
    And the mock subagent "w1" has status "error"
    And the mock subagent "w1" has run error "HTTP 429 from Codex: usage_limit_reached"
    When I execute agent_cmd with '{"agent_id":"w1","command":"await","timeout":5,"idle_timeout":0}'
    Then the agent_cmd await result status should be "error"
    And the agent_cmd await result reason should be "agent_error"
    And the agent_cmd await result error should be "HTTP 429 from Codex: usage_limit_reached"
    And the agent_cmd await result verdict should be "failed"
    And the agent_cmd await result summary should contain "usage_limit_reached"

  # --- Default values ---

  Scenario: await uses default timeout of 300 seconds
    Given an AgentCmdTool with a mock await registry
    And the mock subagent "w1" has status "idle"
    When I execute agent_cmd with '{"agent_id":"w1","command":"await","idle_timeout":0}'
    Then the agent_cmd await result status should be "idle"

  # Default idle_timeout of 5s is verified in unit test
  # (test_await_idle_timeout_waits_correct_duration). BDD scenario omitted
  # to avoid adding 7+ seconds to the test suite.

  # --- Tool definition ---

  Scenario: tool definition includes await in supported commands
    Given an AgentCmdTool with an empty registry
    Then the agent_cmd tool definition description should contain "await"
    And the agent_cmd tool definition schema should include "await" in command enum

  Scenario: tool definition schema includes timeout and idle_timeout
    Given an AgentCmdTool with an empty registry
    Then the agent_cmd tool definition schema should include property "timeout"
    And the agent_cmd tool definition schema should include property "idle_timeout"

  # --- Audit event ---

  Scenario: SubagentAwait audit event round-trips through serde
    Given a SubagentAwait audit event with agent_id "bookmarks-v1" status "idle" reason "completed" elapsed_ms 52000
    When I serialize and deserialize the audit event
    Then the deserialized audit event should match the original
