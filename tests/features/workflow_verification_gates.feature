@wip
Feature: Workflow verification gates (PRD Stage D)
  A step is "done" only when its acceptance gate passes, and a parent does not
  advance on an unverified child result — so plausible-but-wrong intermediate
  state cannot propagate.

  # R-D1 — acceptance gate per step
  Scenario: a step with an acceptance gate is not marked done until the gate passes
    Given a workflow step with an acceptance gate that currently fails
    When the agent attempts to complete the step
    Then the step should not be marked done
    And the rejection reason should mention the acceptance gate

  Scenario: a passing acceptance gate allows the step to complete
    Given a workflow step with an acceptance gate that passes
    When the agent attempts to complete the step
    Then the step should be marked done

  Scenario: a step with no acceptance gate completes as before
    Given a workflow step with no acceptance gate
    When the agent attempts to complete the step
    Then the step should be marked done

  # R-D2 — verify child results
  Scenario: a parent step does not advance on an unverified child result
    Given a parent step that consumes a child result
    And the child result fails the step's acceptance gate
    When the parent attempts to advance past the step
    Then the parent step should not advance
    And the rejection reason should mention verification
