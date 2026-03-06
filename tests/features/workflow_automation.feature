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

  Scenario: Default workflow config has enforce_commit_after_step set to 6
    Given a default workflow config
    Then the workflow config enforce_commit_after_step should be 6

  Scenario: Workflow config with auto_continue disabled
    Given a workflow config with auto_continue false
    Then the workflow config auto_continue should be false

  Scenario: Workflow config with completion_nudge disabled
    Given a workflow config with completion_nudge false
    Then the workflow config completion_nudge should be false

  Scenario: Workflow config with enforce_commit_after_step set to null
    Given a workflow config with enforce_commit_after_step null
    Then the workflow config enforce_commit_after_step should be None

  Scenario: Workflow config with custom enforce_commit_after_step
    Given a workflow config with enforce_commit_after_step 4
    Then the workflow config enforce_commit_after_step should be 4

  Scenario: Workflow config deserializes from JSON with all new fields
    Given a config JSON with workflow auto_continue false and completion_nudge false and enforce_commit_after_step 3
    When I deserialize the workflow config
    Then the workflow config auto_continue should be false
    And the workflow config completion_nudge should be false
    And the workflow config enforce_commit_after_step should be 3

  Scenario: Workflow config deserializes from empty JSON with defaults
    Given an empty config JSON
    When I deserialize the workflow config
    Then the workflow config auto_continue should be true
    And the workflow config completion_nudge should be true
    And the workflow config enforce_commit_after_step should be 6

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

  # ─── Domain: Commit enforcement check ──────────────────────────────────────

  Scenario: Commit blocked when required steps incomplete
    Given a default workflow state
    And enforce_commit_after_step is 6
    When I check if commit is allowed
    Then the commit should be blocked
    And the block reason should contain "step 1"

  Scenario: Commit allowed when required steps complete
    Given a workflow state with steps 1 through 6 checked
    And enforce_commit_after_step is 6
    When I check if commit is allowed
    Then the commit should be allowed

  Scenario: Commit allowed when enforcement disabled
    Given a default workflow state
    And enforce_commit_after_step is None
    When I check if commit is allowed
    Then the commit should be allowed

  Scenario: Commit allowed when enforcement step is 0
    Given a default workflow state
    And enforce_commit_after_step is 0
    When I check if commit is allowed
    Then the commit should be allowed

  Scenario: Commit blocked with partial steps
    Given a workflow state with steps 1 through 4 checked
    And enforce_commit_after_step is 6
    When I check if commit is allowed
    Then the commit should be blocked
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

  Scenario: Workflow tool check_commit action returns blocked when steps incomplete
    Given a workflow tool with default state and enforce_commit_after_step 6
    When I execute the workflow tool with action "check_commit"
    Then the workflow tool result should be an error
    And the workflow tool result should contain "step 1"

  Scenario: Workflow tool check_commit action returns allowed when steps complete
    Given a workflow tool with default state and enforce_commit_after_step 6
    When I execute the workflow tool with action "check" and step 1
    And I execute the workflow tool with action "check" and step 2
    And I execute the workflow tool with action "check" and step 3
    And I execute the workflow tool with action "check" and step 4
    And I execute the workflow tool with action "check" and step 5
    And I execute the workflow tool with action "check" and step 6
    And I execute the workflow tool with action "check_commit"
    Then the workflow tool result should not be an error
    And the workflow tool result should contain "allowed"

  Scenario: Workflow tool check_commit returns allowed when enforcement disabled
    Given a workflow tool with default state and enforce_commit_after_step None
    When I execute the workflow tool with action "check_commit"
    Then the workflow tool result should not be an error
    And the workflow tool result should contain "allowed"

  # ─── System prompt: completion and enforcement annotations ─────────────────

  Scenario: System prompt snippet includes completion message when all done
    Given a workflow state with all 16 steps checked
    When I build the workflow system prompt snippet
    Then the snippet should contain "All steps complete"

  Scenario: System prompt snippet includes enforcement reminder
    Given a default workflow state
    When I build the workflow system prompt snippet with enforce_commit_after_step 6
    Then the snippet should contain "commit"
    And the snippet should contain "steps 1"

  # ─── Config integration: new fields round-trip ─────────────────────────────

  Scenario: Full config with workflow automation fields loads correctly
    Given a config file with workflow auto_continue true and enforce_commit_after_step 4
    When I load the config
    Then the workflow config auto_continue should be true
    And the workflow config enforce_commit_after_step should be 4

  Scenario: Config without new workflow fields uses defaults
    Given a config file with only workflow enabled true
    When I load the config
    Then the workflow config auto_continue should be true
    And the workflow config completion_nudge should be true
    And the workflow config enforce_commit_after_step should be 6
