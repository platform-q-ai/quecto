@wip
Feature: Workflow journaling and resume (PRD Stage C)
  A unit checkpoints each step transition so it can resume from its journal:
  re-running becomes replay, not redo. Orchestration is a pure function of
  (spec + journal).

  # R-C1 — per-step journal
  Scenario: completing a step appends a journal entry
    Given a workflow engine on a 5-step template with a fresh run
    When step 1 is completed
    Then the persisted journal should contain a "completed" entry for step 1

  Scenario: a failed step is recorded in the journal
    Given a workflow engine on a 5-step template with a fresh run
    When step 1 is completed
    And step 2 is recorded as failed
    Then the persisted journal should contain a "failed" entry for step 2

  # R-C2 — resume from journal
  Scenario: re-instantiating from spec and journal resumes at the first incomplete step
    Given a persisted workflow run with steps 1 and 2 completed of 5
    When the unit is re-instantiated from its spec and journal
    Then the current step should be step 3

  Scenario: completed steps are not re-run on resume
    Given a persisted workflow run with steps 1 and 2 completed of 5
    When the unit is re-instantiated from its spec and journal
    Then steps 1 and 2 should remain marked done

  # R-C3 — deterministic replay
  Scenario: resume is deterministic for the same spec and journal
    Given a persisted workflow run with steps 1 and 2 completed of 5
    When the unit is re-instantiated twice from the same spec and journal
    Then both instances should report the same current step and progress
