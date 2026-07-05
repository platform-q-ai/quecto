@done @workflow
Feature: Workflow tool behavior
  As a UDS agent using the workflow tool
  I want workflow actions to mutate state, emit snapshots, and enforce guards predictably
  So that clients can trust the workflow stream and guarded command feedback

  Scenario: Selecting a template emits an active workflow snapshot with issue context
    Given a workflow tool for a three-step guarded template
    When I run workflow action '{"action":"select_template","template":"wave","issueNumber":1028,"issueTitle":"BDD coverage"}'
    Then the workflow tool result should not be an error
    And the workflow tool result should contain "Selected workflow template 'wave'"
    And the last workflow event should have mode "active"
    And the last workflow event should have active issue number 1028 and title "BDD coverage"
    And the last workflow event current step should be 1 with key "plan"

  Scenario: Read-only workflow actions do not emit workflow_state events
    Given a workflow tool for a three-step guarded template
    When I run workflow action '{"action":"status"}'
    Then the workflow tool result should not be an error
    And no workflow event should be emitted

  Scenario: Ordering violations fail without emitting a new workflow snapshot
    Given a workflow tool for a three-step guarded template
    And the workflow template "wave" is selected
    And workflow events are cleared
    When I run workflow action '{"action":"check","step":2}'
    Then the workflow tool result should be an error
    And the workflow tool result should contain "complete step 1"
    And no workflow event should be emitted

  Scenario: Matching workflow guards block commands until prerequisite steps are complete
    Given a workflow tool for a three-step guarded template
    And the workflow template "wave" is selected
    When I run workflow action '{"action":"check_guards","command":"cargo test -p quecto-agentic-harness"}'
    Then the workflow tool result should be an error
    And the workflow tool result should contain "finish workflow tests first"
    And the workflow tool result should contain "Complete step 1"

  Scenario: Matching workflow guards allow commands after prerequisite steps are complete
    Given a workflow tool for a three-step guarded template
    And the workflow template "wave" is selected
    And workflow step 1 is checked through the tool
    And workflow step 2 is checked through the tool
    When I run workflow action '{"action":"check_guards","command":"cargo test -p quecto-agentic-harness"}'
    Then the workflow tool result should not be an error
    And the workflow tool result should contain "All workflow guards for command are satisfied"
