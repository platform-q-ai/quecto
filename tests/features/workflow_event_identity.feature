@done
Feature: Workflow event identity and forwarding (PRD Stage B)
  Identity-tagged events let any consumer reconstruct the unit tree and each
  unit's workflow from the event stream alone, without polling child sockets.

  # R-B1 — identity on every workflow/subagent event
  Scenario: workflow_state events carry agent_id and parent_id
    Given a workflow agent "root" with no parent
    When the agent emits a workflow_state event
    Then the workflow_state event should include agent_id "root"
    And the workflow_state event should include a parent_id field

  # R-B3 — SubagentInfo gains workflow + parent_id
  Scenario: a subagent entry carries parent_id and an optional workflow snapshot
    Given a parent agent "root"
    And a subagent "child" spawned by "root" running a workflow at 2 of 5 steps
    When the parent builds its subagent info list
    Then the subagent entry for "child" should include parent_id "root"
    And the subagent entry for "child" should include a workflow snapshot of 2 of 5 steps

  # R-B2 — children forward events to the parent
  Scenario: a child's workflow_state is forwarded to the parent's event stream
    Given a parent agent "root" with subagent "child"
    When "child" advances its workflow
    Then "root"'s event stream should receive a workflow_state event tagged agent_id "child" parent_id "root"

  # R-B4 — tree reconstructable from the stream alone
  Scenario: the unit tree is reconstructable from the event stream alone
    Given an event stream with identity-tagged workflow_state events for "root", "child" under "root", and "grandchild" under "child"
    When a consumer reconstructs the unit tree from the stream
    Then the tree should place "grandchild" under "child" under "root"
