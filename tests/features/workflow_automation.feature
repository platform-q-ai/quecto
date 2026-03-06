@wip
Feature: Workflow automation — auto-continue, completion nudge, enforcement, persistence
  As an agent operator
  I want the workflow tool to automate step progression and enforce discipline
  So that the agent follows the BDD/TDD process without manual nudging

  # ─── Domain: WorkflowConfig extensions ──────────────────────────────────────

  Scenario: Default workflow config has auto_continue enabled
    Given a default workflow config
    Then the workflow config auto_continue should be true

  Scenario: Default workflow config has completion_nudge enabled
    Given a default workflow config
    Then the workflow config completion_nudge should be true

  Scenario: Default workflow config has no guards
    Given a default workflow config
    Then the workflow config should have 0 guards

  Scenario: Workflow config with auto_continue disabled
    Given a workflow config with auto_continue false
    Then the workflow config auto_continue should be false

  Scenario: Workflow config with completion_nudge disabled
    Given a workflow config with completion_nudge false
    Then the workflow config completion_nudge should be false

  Scenario: Workflow config with guards
    Given a workflow config with guards:
      | commands    | before_step | message        |
      | git commit  | 7           | Not yet.       |
    Then the workflow config should have 1 guards

  Scenario: Workflow config deserializes from JSON with all new fields
    Given a config JSON with workflow auto_continue false and completion_nudge false
    When I deserialize the workflow config
    Then the workflow config auto_continue should be false
    And the workflow config completion_nudge should be false

  Scenario: Workflow config deserializes from empty JSON with defaults
    Given an empty config JSON
    When I deserialize the workflow config
    Then the workflow config auto_continue should be true
    And the workflow config completion_nudge should be true
    And the workflow config should have 0 guards

  # ─── Domain: Auto-continue nudge generation ─────────────────────────────────

  Scenario: Auto-continue nudge generated when some steps checked
    Given a default workflow state with step 1 checked
    When I generate the auto_continue nudge
    Then the nudge should contain "step 2"

  Scenario: No auto-continue nudge when all steps complete
    Given a workflow state with all 16 steps checked
    When I generate the auto_continue nudge
    Then the nudge should be None

  Scenario: Auto-continue nudge for fresh state points to step 1
    Given a default workflow state
    When I generate the auto_continue nudge
    Then the nudge should contain "step 1"

  # ─── Domain: Completion nudge generation ────────────────────────────────────

  Scenario: Completion nudge generated when all steps done
    Given a workflow state with all 16 steps checked
    When I generate the completion nudge
    Then the nudge should contain "Close"
    And the nudge should contain "next"
    And the nudge should contain "issue"

  Scenario: No completion nudge when steps incomplete
    Given a default workflow state with step 1 checked
    When I generate the completion nudge
    Then the nudge should be None

  # ─── Domain: Step threshold check ───────────────────────────────────────────

  Scenario: Steps incomplete fails threshold check
    Given a default workflow state
    When I check steps complete before step 7
    Then the check should fail
    And the block reason should contain "step 1"

  Scenario: Steps complete passes threshold check
    Given a workflow state with steps 1 through 6 checked
    When I check steps complete before step 7
    Then the check should pass

  Scenario: Threshold 0 always passes
    Given a default workflow state
    When I check steps complete before step 0
    Then the check should pass

  Scenario: Partial steps fail threshold check
    Given a workflow state with steps 1 through 4 checked
    When I check steps complete before step 7
    Then the check should fail
    And the block reason should contain "step 5"

  # ─── Domain: Workflow state persistence (serialization) ────────────────────

  Scenario: Workflow state serializes to persistable format
    Given a default workflow state with step 1 checked
    And the active issue is 42 "My feature"
    When I serialize the workflow state
    Then the serialized state should contain step 1 as done
    And the serialized state should contain step 2 as not done
    And the serialized state should contain issue 42 "My feature"

  Scenario: Workflow state round-trips through serialization
    Given a default workflow state with step 1 checked
    And the active issue is 42 "My feature"
    When I serialize and deserialize the workflow state
    Then step 1 should be checked
    And step 2 should be unchecked
    And the active issue should be 42 "My feature"

  Scenario: Empty workflow state round-trips correctly
    Given a default workflow state
    When I serialize and deserialize the workflow state
    Then all steps should be unchecked
    And the active issue should be None

  # ─── Tool: Commit enforcement via workflow tool ────────────────────────────

  Scenario: Workflow tool check_commit action returns blocked when guards unsatisfied
    Given a workflow tool with default state and guard before step 7
    When I execute the workflow tool with action "check_commit"
    Then the workflow tool result should be an error
    And the workflow tool result should contain "step 1"

  Scenario: Workflow tool check_commit action returns satisfied when guards met
    Given a workflow tool with default state and guard before step 7
    When I execute the workflow tool with action "check" and step 1
    And I execute the workflow tool with action "check" and step 2
    And I execute the workflow tool with action "check" and step 3
    And I execute the workflow tool with action "check" and step 4
    And I execute the workflow tool with action "check" and step 5
    And I execute the workflow tool with action "check" and step 6
    And I execute the workflow tool with action "check_commit"
    Then the workflow tool result should not be an error
    And the workflow tool result should contain "satisfied"

  Scenario: Workflow tool check_commit returns satisfied when no guards
    Given a workflow tool with default state and no guards
    When I execute the workflow tool with action "check_commit"
    Then the workflow tool result should not be an error
    And the workflow tool result should contain "satisfied"

  # ─── System prompt: completion and enforcement annotations ─────────────────

  Scenario: System prompt snippet includes completion message when all done
    Given a workflow state with all 16 steps checked
    When I build the workflow system prompt snippet
    Then the snippet should contain "All steps complete"

  Scenario: System prompt snippet includes guard reminder
    Given a default workflow state
    When I build the workflow system prompt snippet with guards
    Then the snippet should contain "Guard"
    And the snippet should contain "git commit"

  # ─── Config integration: new fields round-trip ─────────────────────────────

  Scenario: Full config with workflow automation fields loads correctly
    Given a config file with workflow auto_continue true and guards configured
    When I load the config
    Then the workflow config auto_continue should be true
    And the workflow config should have 1 guards

  Scenario: Config without new workflow fields uses defaults
    Given a config file with only workflow enabled true
    When I load the config
    Then the workflow config auto_continue should be true
    And the workflow config completion_nudge should be true
    And the workflow config should have 0 guards
