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

  Scenario: Task validation explains the required acceptance type
    When a swarm member supplies acceptance as a string
    Then the swarm result should be an error
    And the swarm result should contain "list[str]"

  Scenario: Approval waits block a task without stopping the swarm
    When a swarm task awaits master approval
    Then the swarm run status is "running"
    And the swarm task status is "blocked"
    When the approved swarm task is completed
    Then the swarm run status is "running"
    And the swarm task status is "submitted"

  Scenario: Workflow-enabled container launches are rejected
    When a workflow-enabled swarm container is requested
    Then the swarm result should be an error
    And the swarm result should contain "workflow is unavailable for swarm agents"

  Scenario: Rejected wake delivery preserves the durable message and reports a warning
    When a swarm message recipient rejects its wake hint
    Then the swarm result should not be an error
    And the swarm result should contain "wake hint failed for worker"
    And the rejected wake still leaves the message in the recipient inbox

  Scenario: Reviewed submissions cannot change under an existing claim
    When a swarm member submits its work
    And the member replaces submitted evidence under the same claim
    Then the swarm result should be an error
    And the swarm result should contain "submitted evidence is immutable"

  Scenario: Criteria amendments retain the original definition of done
    When the swarm coordinator changes only the done criteria
    Then the swarm audit retains both complete contracts

  Scenario Outline: Completed invocations settle ordinary subprocesses
    Given swarm Python is permitted to create subprocesses
    When a "<mode>" swarm interpreter returns before its ordinary child
    Then the completed swarm invocation has stopped its child

    Examples:
      | mode       |
      | foreground |
      | background |

  Scenario: Claimed work does not wake an idle peer
    Given an idle swarm peer with an unavailable endpoint
    When the coordinator creates and immediately claims a task
    Then no swarm wake delivery is attempted
