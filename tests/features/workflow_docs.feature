@docs @wip
Feature: Workflow reference docs
  As a maintainer
  I want the workflow docs and examples to match the built-in workflow tooling
  So that operators configure and understand the agent correctly

  @docs
  Scenario: README documents the 16-step reference workflow and guard rules
    When I read the repository file "README.md"
    Then the output should contain "5 - Refactor (perf, security, clean arch)"
    And the output should contain "6 - Ensure tests still pass (GREEN)"
    And the output should contain "15 - Merge"
    And the output should contain "16 - Move to local master and pull"
    And the output should contain "Complete RED-GREEN-REFACTOR (steps 1-6) before committing."
    And the output should not contain "\"guard_commit\": true"
    And the output should not contain "\"enforce_commit_after_step\": 6"

  @docs
  Scenario: Workflow guide documents the full reference example and in-memory runtime state
    When I read the repository file "docs/workflow.md"
    Then the output should contain "Move to local master and pull"
    And the output should contain "Complete RED-GREEN-REFACTOR (steps 1-6) before committing."
    And the output should contain "Complete code review (steps 10-14) before merging."
    And the output should contain "Workflow state is stored in-memory"
    And the output should contain "stored in-memory for the lifetime of the agent process"
    And the output should contain "workflow guards can be circumvented by"
    And the output should not contain "the workflow state is included in the session file"
