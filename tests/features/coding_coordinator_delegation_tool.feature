@wip
Feature: Coordinator Delegation Tool
  As the LLM agent
  I want a coding_job tool that delegates to the coordinator via file-based IPC
  So that the coordinator runs as a separate process and the main agent stays responsive

  The CoordinatorDelegationTool replaces the inline CodingJobTool for the
  main agent. It writes command JSON to the coordinator inbox, polls the
  outbox for responses, and checks for proactive notifications. The tool
  name and schema remain "coding_job" for backward compatibility.

  # --- Tool definition ---

  Scenario: Delegation tool has the same name and schema as CodingJobTool
    Given a coordinator delegation tool with a mock IPC
    Then the delegation tool name should be "coding_job"
    And the delegation tool description should mention coding jobs
    And the delegation tool schema should require an "action" field

  # --- Command delegation ---

  Scenario: Run action writes command to inbox and reads response from outbox
    Given a coordinator delegation tool with a mock IPC
    And the mock IPC will respond with ok true and body {"run_id":"r1","job_id":"j1","state":"queued"}
    When I execute the delegation tool with action "run" and payload {"goal":"Fix bug","repo":"test-repo","base_ref":"main"}
    Then the mock IPC inbox should have received a command with action "run"
    And the delegation tool result should not be an error
    And the delegation tool result should contain "job_id"

  Scenario: Status action writes command to inbox and reads response
    Given a coordinator delegation tool with a mock IPC
    And the mock IPC will respond with ok true and body {"job_id":"j1","run_id":"r1","state":"running","progress":50}
    When I execute the delegation tool with action "status" and payload {"job_id":"j1"}
    Then the delegation tool result should not be an error
    And the delegation tool result should contain "running"

  Scenario: Cancel action delegates to coordinator
    Given a coordinator delegation tool with a mock IPC
    And the mock IPC will respond with ok true and body {"job_id":"j1","state":"canceled"}
    When I execute the delegation tool with action "cancel" and payload {"job_id":"j1"}
    Then the delegation tool result should not be an error
    And the delegation tool result should contain "canceled"

  Scenario: List action delegates to coordinator
    Given a coordinator delegation tool with a mock IPC
    And the mock IPC will respond with ok true and body {"jobs":[]}
    When I execute the delegation tool with action "list" and payload {}
    Then the delegation tool result should not be an error
    And the delegation tool result should contain "jobs"

  # --- Error handling ---

  Scenario: IPC error response is returned as tool error
    Given a coordinator delegation tool with a mock IPC
    And the mock IPC will respond with ok false and error "not_found"
    When I execute the delegation tool with action "status" and payload {"job_id":"missing"}
    Then the delegation tool result should be an error
    And the delegation tool result should contain "not_found"

  Scenario: IPC timeout is returned as tool error
    Given a coordinator delegation tool with a mock IPC that times out
    When I execute the delegation tool with action "status" and payload {"job_id":"j1"}
    Then the delegation tool result should be an error
    And the delegation tool result should contain "timeout"

  Scenario: Invalid JSON input returns an error
    Given a coordinator delegation tool with a mock IPC
    When I execute the delegation tool with raw input "not valid json"
    Then the delegation tool result should be an error
    And the delegation tool result should contain "invalid JSON"

  Scenario: Missing action field returns an error
    Given a coordinator delegation tool with a mock IPC
    When I execute the delegation tool with raw input "{}"
    Then the delegation tool result should be an error
    And the delegation tool result should contain "action"

  Scenario: Unknown action returns an error
    Given a coordinator delegation tool with a mock IPC
    When I execute the delegation tool with raw input '{"action":"explode"}'
    Then the delegation tool result should be an error
    And the delegation tool result should contain "unknown action"

  # --- Notification checking ---

  Scenario: Delegation tool includes notifications in status response
    Given a coordinator delegation tool with a mock IPC
    And the mock IPC has pending notifications:
      | type       | job_id | detail              |
      | job_failed | j1     | OOM killed after 2h |
    And the mock IPC will respond with ok true and body {"job_id":"j1","run_id":"r1","state":"failed"}
    When I execute the delegation tool with action "status" and payload {"job_id":"j1"}
    Then the delegation tool result should not be an error
    And the delegation tool result should contain "notifications"
    And the delegation tool result should contain "job_failed"
