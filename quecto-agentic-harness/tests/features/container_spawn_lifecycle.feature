Feature: Container-backed spawn lifecycle parity
  Container-backed agents use the same parent control model as local subagents while retaining script-reported runtime metadata and pushed liveness.

  # AC7
  @wip @issue-1369-ac7-ac9
  Scenario: Container-backed agents preserve local control parity
    Given a local subagent has completed a workflow run with transcript history
    And a container-backed subagent has completed the same workflow run with transcript history
    When the parent compares lifecycle controls for both subagents
    Then agent_cmd messages, workflow state, status, transcript, kill, await, and cleanup are equivalent

  # AC8
  @wip @issue-1369-ac7-ac9
  Scenario: Script-reported metadata is retained without runtime inference
    Given a container create script reports environment metadata
    When the container entry is recorded
    Then the recorded container metadata exactly matches the script output
    And no Docker or runtime-specific fields are inferred by Quecto core

  # AC9
  @wip @issue-1369-ac7-ac9
  Scenario: Socket EOF updates status and pending awaits after one post-mortem inspect
    Given a container-backed subagent has a liveness connection and a pending await
    When the liveness connection receives EOF
    Then the subagent is marked exited from the pushed liveness signal
    And exactly one post-mortem inspect is requested
    And the pending await completes without polling
