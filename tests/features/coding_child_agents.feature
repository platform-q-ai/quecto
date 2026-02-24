@pending
Feature: Child Agent Spawn Flow
  As the coding runtime coordinator
  I want to manage child agent spawning requested by workers
  So that specialized tasks like security reviews run safely under policy control

  Workers emit spawn.request events. The coordinator validates against policy
  (allowlist, depth, budget) and either approves or denies. Approved child
  agents run with the same isolation model. Results are routed back to the
  parent worker and aggregated for the main agent.

  Background:
    Given a coding coordinator with child agent policy:
      | allow_types       | security-reviewer, performance-reviewer, architecture-reviewer, documentation-updater |
      | max_depth          | 1                                                                                     |
      | max_spawns_per_job | 3                                                                                     |
    And a coding job in state "running"

  # --- Spawn request approval ---

  Scenario: Worker requests an allowlisted child agent and coordinator approves
    When the worker emits a "spawn.request" event with:
      | request_id  | s1                |
      | agent_type  | security-reviewer |
      | scope       | current diff      |
    Then a "spawn.decision" event should be emitted with approved true
    And the child agent should be launched

  Scenario: Coordinator denies a non-allowlisted agent type
    When the worker emits a "spawn.request" event with:
      | request_id  | s2                |
      | agent_type  | unknown-agent     |
      | scope       | full repo         |
    Then a "spawn.decision" event should be emitted with approved false
    And the reason should indicate the agent type is not allowed
    And no child agent should be launched

  # --- Spawn limits ---

  Scenario: Coordinator denies spawn when per-job limit is reached
    Given the job has already spawned 3 child agents
    When the worker emits a 4th "spawn.request" event
    Then a "spawn.decision" event should be emitted with approved false
    And the reason should indicate the per-job spawn limit is reached

  Scenario: Coordinator denies spawn when max depth would be exceeded
    Given the current job is already a child agent at depth 1
    When the worker emits a "spawn.request" event
    Then a "spawn.decision" event should be emitted with approved false
    And the reason should indicate the max spawn depth is reached

  # --- Spawn result routing ---

  Scenario: Child agent completes successfully and result is routed back
    Given a child agent "security-reviewer" was approved and launched
    When the child agent completes with state "succeeded" and summary "2 medium findings"
    Then a "spawn.result" event should be emitted with:
      | request_id | s1                |
      | state      | succeeded         |
      | summary    | 2 medium findings |
    And the parent worker should receive the child result
    And the main agent should receive an updated job summary

  Scenario: Child agent fails and failure is reported
    Given a child agent "performance-reviewer" was approved and launched
    When the child agent fails with state "failed"
    Then a "spawn.result" event should be emitted with state "failed"
    And the parent worker should be notified of the failure

  Scenario: Child agent produces artifacts that are tracked
    Given a child agent "security-reviewer" was approved and launched
    When the child agent creates artifact "security_review.md"
    Then a "spawn.result" event should include artifact_refs containing "security_review.md"
    And the artifact should be accessible in the parent job's artifact directory

  # --- Child agent isolation ---

  Scenario: Child agent cannot call GitHub APIs directly
    Given a child agent "security-reviewer" is running
    When the child agent attempts to emit a "publish.request" event
    Then the coordinator should reject the event
    And the child agent should receive an error

  Scenario: Child agent follows same nsjail isolation as parent worker
    Given a child agent "documentation-updater" was approved and launched
    Then the child agent should run inside nsjail with the same resource limits
    And the child agent should have a writable mount only for its own job directory

  # --- Audit trail ---

  Scenario: All spawn events are persisted in event log
    When a worker requests a child agent and it completes
    Then the event log should contain "spawn.request", "spawn.decision", and "spawn.result" events
    And an "artifact.created" event should be emitted with artifact_type "spawn_log"

  # --- Deduplication ---

  Scenario: Coordinator deduplicates equivalent spawn requests
    When the worker emits two "spawn.request" events with identical agent_type and scope
    Then only one child agent should be launched
    And the second request should receive the result of the first

  # --- Expected output routing ---

  Scenario: Spawn request includes expected_output for result routing
    When the worker emits a "spawn.request" event with:
      | request_id      | s3                        |
      | agent_type      | security-reviewer         |
      | scope           | current diff              |
      | expected_output | security_findings.json    |
    Then a "spawn.decision" event should be emitted with approved true
    And the child agent should receive the expected_output specification

  # --- Timeout handling ---

  Scenario: Child agent that exceeds timeout is terminated
    Given a child agent "architecture-reviewer" was approved and launched
    When the child agent exceeds the configured timeout
    Then the child agent should be terminated
    And a "spawn.result" event should be emitted with state "failed"
    And the summary should indicate timeout

  # --- Cancel propagation ---

  Scenario: Canceling parent job cancels running child agents
    Given a child agent "security-reviewer" is running
    When the parent job is canceled
    Then the child agent should be terminated
    And a "spawn.result" event should be emitted with state "canceled"

  Scenario: Child agent canceled state is terminal
    Given a child agent "security-reviewer" was canceled
    When the coordinator checks the spawn result
    Then the spawn.result state should be "canceled"
    And no further events should be emitted for this child agent

  # --- Concurrent spawns ---

  Scenario: Multiple child agents can run concurrently within limits
    When the worker emits "spawn.request" events for:
      | request_id | agent_type             |
      | s1         | security-reviewer      |
      | s2         | performance-reviewer   |
      | s3         | architecture-reviewer  |
    Then all 3 spawn.decision events should have approved true
    And all 3 child agents should be launched concurrently

  # --- Depth chain enforcement ---

  Scenario: Child agent spawn request is denied when it would create depth 2
    Given a child agent "security-reviewer" is running at depth 1
    When the child agent emits a "spawn.request" for another child agent
    Then a "spawn.decision" event should be emitted with approved false
    And the reason should indicate max depth 1 would be exceeded

  # --- Unknown request_id handling ---

  Scenario: Spawn result with unknown request_id is rejected
    When a "spawn.result" event arrives with request_id "unknown_req"
    Then the coordinator should log a warning
    And the event should be discarded

  # --- Allowlist coverage ---

  Scenario: documentation-updater is approved as allowlisted type
    When the worker emits a "spawn.request" event with:
      | request_id | s4                    |
      | agent_type | documentation-updater |
      | scope      | changed files         |
    Then a "spawn.decision" event should be emitted with approved true
