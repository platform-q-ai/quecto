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

  # ─── Cache-safe prompting (#1113): guidance travels in tool results ─────────
  # The system prompt is static for the whole session, so the tool results of
  # select_template, check, and status must hand the model the current/next
  # step's label and guidance exactly when it advances.

  @cache-safe-prompt
  Scenario: Selecting a template returns the current step and its guidance in the tool result
    Given a workflow tool for a three-step guarded template
    When the model selects the workflow template "wave"
    Then the workflow tool result should not be an error
    And the workflow tool result should carry the current step's label and guidance

  @cache-safe-prompt
  Scenario: Checking a step returns the next step and its guidance in the tool result
    Given a workflow tool for a three-step guarded template
    And the workflow template "wave" is selected
    When the model checks off workflow step 1
    Then the workflow tool result should not be an error
    And the workflow tool result should carry the next step's label and guidance

  # Skip advances the current step exactly like check, so its result must
  # hand the model the next step's label and guidance too (#1113 AC2).
  @cache-safe-prompt
  Scenario: Skipping a step returns the next step and its guidance in the tool result
    Given a workflow tool for a three-step guarded template
    And the workflow template "wave" is selected
    When the model skips workflow step 1
    Then the workflow tool result should not be an error
    And the workflow tool result should carry the next step's label and guidance

  # Uncheck can move the current step BACKWARDS — its result must re-orient
  # the model on the step the workflow rewound to (#1113 AC2).
  @cache-safe-prompt
  Scenario: Unchecking a step returns the rewound current step and its guidance in the tool result
    Given a workflow tool for a three-step guarded template
    And the workflow template "wave" is selected
    And workflow step 1 is checked through the tool
    When the model unchecks workflow step 1
    Then the workflow tool result should not be an error
    And the workflow tool result should carry the current step's label and guidance

  # NOTE: regression PIN of pre-#1113 status_text behavior (the channel #1113
  # leans on), not proof of new #1113 work — falsifiable #1113 coverage lives
  # in the select_template/check/skip/uncheck handoff scenarios.
  @cache-safe-prompt
  Scenario: Requesting the status returns the current step and its guidance in the tool result
    Given a workflow tool for a three-step guarded template
    And the workflow template "wave" is selected
    When the model requests the workflow status
    Then the workflow tool result should not be an error
    And the workflow tool result should carry the current step's label and guidance

  @cache-safe-prompt
  Scenario: The workflow tool description advertises template selection
    Given a workflow tool for a three-step guarded template
    When I read the workflow tool definition
    Then the definition description should advertise the list_templates and select_template actions

  Scenario: Matching workflow guards allow commands after prerequisite steps are complete
    Given a workflow tool for a three-step guarded template
    And the workflow template "wave" is selected
    And workflow step 1 is checked through the tool
    And workflow step 2 is checked through the tool
    When I run workflow action '{"action":"check_guards","command":"cargo test -p quecto-agentic-harness"}'
    Then the workflow tool result should not be an error
    And the workflow tool result should contain "All workflow guards for command are satisfied"
