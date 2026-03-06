@wip
Feature: Built-in workflow tool
  As an agent operator
  I want a workflow tool that tracks development workflow progress
  So that the agent follows BDD/TDD Red-Green-Refactor steps consistently

  # ─── Domain: WorkflowState ──────────────────────────────────────────────────

  Scenario: Default workflow state has 16 unchecked steps
    Given a default workflow state
    Then the workflow state should have 16 steps
    And all steps should be unchecked
    And the active issue should be None

  Scenario: Check marks a step as done
    Given a default workflow state
    When I check step 1
    Then step 1 should be checked
    And step 2 should be unchecked

  Scenario: Uncheck marks a step as not done
    Given a default workflow state
    When I check step 1
    And I uncheck step 1
    Then step 1 should be unchecked

  Scenario: Check enforces step ordering
    Given a default workflow state
    When I try to check step 3
    Then the check should fail with "complete step 1 first"

  Scenario: Check allows the next uncompleted step
    Given a default workflow state
    When I check step 1
    And I check step 2
    Then step 2 should be checked

  Scenario: Skip bypasses ordering enforcement
    Given a default workflow state
    When I skip step 5
    Then step 5 should be checked
    And step 4 should be unchecked

  Scenario: Reset clears all steps and active issue
    Given a default workflow state
    When I check step 1
    And I set issue 42 "My feature"
    And I reset the workflow
    Then all steps should be unchecked
    And the active issue should be None

  Scenario: Set issue records number and title
    Given a default workflow state
    When I set issue 42 "My feature"
    Then the active issue should be 42 "My feature"

  Scenario: Clear issue removes the active issue
    Given a default workflow state
    When I set issue 42 "My feature"
    And I clear the active issue
    Then the active issue should be None

  Scenario: Progress reports done count, total, and percent
    Given a default workflow state
    When I check step 1
    And I check step 2
    Then the progress should be 2 done out of 16 total with percent 12

  Scenario: Check out-of-range step returns error
    Given a default workflow state
    When I try to check step 0
    Then the check should fail with "invalid step"

  Scenario: Check out-of-range step high returns error
    Given a default workflow state
    When I try to check step 17
    Then the check should fail with "invalid step"

  Scenario: Uncheck out-of-range step returns error
    Given a default workflow state
    When I try to uncheck step 0
    Then the uncheck should fail with "invalid step"

  # ─── Domain: WorkflowConfig ────────────────────────────────────────────────

  Scenario: Default workflow config has enabled false and 16 default steps
    Given a default workflow config
    Then the workflow config should not be enabled
    And the workflow config should have 16 steps
    And the first step should be id 1 label "Update Scenarios / Add new features" phase "red"
    And the last step should be id 16 label "Move to local master and pull" phase "ci_cd"

  Scenario: Workflow config can be disabled
    Given a workflow config with enabled false
    Then the workflow config should not be enabled

  # ─── Config integration ────────────────────────────────────────────────────

  Scenario: Config deserializes workflow section
    Given a config file with workflow enabled and custom steps
    When I load the config
    Then the workflow config should be enabled

  Scenario: Config without workflow section uses defaults
    Given a config file without workflow section
    When I load the config
    Then the workflow config should not be enabled
    And the workflow config should have 16 steps

  # ─── Tool: WorkflowTool ────────────────────────────────────────────────────

  Scenario: Workflow tool status returns all steps and progress
    Given a workflow tool with default state
    When I execute the workflow tool with action "status"
    Then the workflow tool result should not be an error
    And the workflow tool result should contain "1."
    And the workflow tool result should contain "0/16"

  Scenario: Workflow tool check marks a step done
    Given a workflow tool with default state
    When I execute the workflow tool with action "check" and step 1
    Then the workflow tool result should not be an error
    And the workflow tool result should contain "checked"

  Scenario: Workflow tool uncheck marks a step not done
    Given a workflow tool with default state
    When I execute the workflow tool with action "check" and step 1
    And I execute the workflow tool with action "uncheck" and step 1
    Then the workflow tool result should not be an error
    And the workflow tool result should contain "unchecked"

  Scenario: Workflow tool reset clears all steps
    Given a workflow tool with default state
    When I execute the workflow tool with action "check" and step 1
    And I execute the workflow tool with action "reset"
    Then the workflow tool result should not be an error
    And the workflow tool result should contain "reset"

  Scenario: Workflow tool skip bypasses ordering
    Given a workflow tool with default state
    When I execute the workflow tool with action "skip" and step 5
    Then the workflow tool result should not be an error
    And the workflow tool result should contain "skipped"

  Scenario: Workflow tool set_issue records issue
    Given a workflow tool with default state
    When I execute the workflow tool with action "set_issue" and issue 42 "My feature"
    Then the workflow tool result should not be an error
    And the workflow tool result should contain "#42"

  Scenario: Workflow tool clear_issue removes issue
    Given a workflow tool with default state
    When I execute the workflow tool with action "set_issue" and issue 42 "My feature"
    And I execute the workflow tool with action "clear_issue"
    Then the workflow tool result should not be an error
    And the workflow tool result should contain "cleared"

  Scenario: Workflow tool unknown action returns error
    Given a workflow tool with default state
    When I execute the workflow tool with action "unknown"
    Then the workflow tool result should be an error
    And the workflow tool result should contain "unknown action"

  Scenario: Workflow tool missing action returns error
    Given a workflow tool with default state
    When I execute the workflow tool with empty arguments
    Then the workflow tool result should be an error
    And the workflow tool result should contain "action"

  Scenario: Workflow tool definition has correct name and schema
    Given a workflow tool with default state
    Then the workflow tool definition name should be "workflow"
    And the workflow tool definition should have a parameters schema

  # ─── UDS event emission ─────────────────────────────────────────────────────

  Scenario: Workflow tool emits workflow_state event on check
    Given a workflow tool with event emitter
    When I execute the workflow tool with action "check" and step 1
    Then a workflow_state event should have been emitted
    And the event should contain "steps"
    And the event should contain "progress"

  Scenario: Workflow tool emits workflow_state event on reset
    Given a workflow tool with event emitter
    When I execute the workflow tool with action "reset"
    Then a workflow_state event should have been emitted

  Scenario: Workflow tool emits workflow_state event on set_issue
    Given a workflow tool with event emitter
    When I execute the workflow tool with action "set_issue" and issue 42 "My feature"
    Then a workflow_state event should have been emitted
    And the event should contain "activeIssue"

  # ─── System prompt injection ────────────────────────────────────────────────

  Scenario: Workflow progress is included in system prompt
    Given a default workflow state with step 1 checked
    When I build the workflow system prompt snippet
    Then the snippet should contain "1/16"
    And the snippet should contain "1."
    And the snippet should contain "CURRENT STEP"

  Scenario: Workflow with active issue includes issue in system prompt
    Given a default workflow state with issue 42 "My feature"
    When I build the workflow system prompt snippet
    Then the snippet should contain "#42"
    And the snippet should contain "My feature"
