@done
Feature: Workflow auto-continue and completion nudge (#562)
  As an agent orchestrator
  I want agents to self-drive through workflow steps
  So that I can manage hundreds of subagents without manual nudging

  Scenario: Auto-continue nudge message generated when steps incomplete
    Given a workflow with 16 steps and 4 completed
    When auto_continue_nudge is called
    Then the nudge message should mention step 5
    And the nudge message should not be empty


  Scenario: No nudge when all steps complete
    Given a workflow with 16 steps all completed
    When auto_continue_nudge is called
    Then the result should be None

  Scenario: Completion nudge when all steps done
    Given a workflow with 16 steps all completed
    When completion_nudge is called
    Then the nudge message should mention closing the issue
    And the nudge message should mention picking the next issue

  Scenario: No completion nudge when steps incomplete
    Given a workflow with 16 steps and 4 completed
    When completion_nudge is called
    Then the result should be None

  Scenario: Workflow state event emitted on step check
    Given a workflow tool with event emitter
    When I execute the workflow tool with action "check" and step 1
    Then a workflow_state event should have been emitted
    And the event should contain "progress"
