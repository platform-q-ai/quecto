@done @swarm
Feature: Container swarm coordination
  As a supervising parent
  I want a bounded shared workbench with verifiable results
  So that workers coordinate without losing ownership or claiming false success

  Background:
    Given a swarm workspace

  Scenario: Goal and tasks survive fresh Python executions
    When a swarm member creates an acceptance task
    Then a later swarm execution sees the acceptance task

  Scenario: Unmet dependencies prevent ownership
    When a swarm member claims work with an unmet dependency
    Then the swarm result should be an error
    And the swarm result should contain "unmet dependencies"

  Scenario: Submitting evidence does not accept completion
    When a swarm member submits its work
    Then the swarm task status is "submitted"

  Scenario: Empty board does not satisfy done criteria
    When the swarm coordinator attempts completion without evidence
    Then the swarm result should be an error
    And the swarm result should contain "completion requires accepted evidence"

  Scenario: Cancellation keeps a readable partial board
    When a swarm member creates an acceptance task
    And the swarm parent cancels the run
    Then the swarm run status is "cancelled"
    And the swarm summary retains one task

  Scenario: Shared checkout reservations are all or nothing
    When swarm members contend for an overlapping file set
    Then only the first swarm file set is owned

  Scenario: Completed dependent work can be revalidated at the final revision
    When the coordinator completes dependent tasks at different revisions
    And revalidates earlier work with fresh final revision evidence
    Then the swarm run status is "succeeded"
