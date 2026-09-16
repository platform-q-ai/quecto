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
    Then the swarm run is paused holding "succeeded"

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

  Scenario: Workflow-enabled swarm worker launches are rejected
    When a swarm participant requests a workflow-enabled worker
    Then the swarm result should be an error
    And the swarm result should contain "workflow is unavailable for swarm agents"

  Scenario: Ordinary container launches keep workflow eligibility
    When a workflow-enabled ordinary container is requested
    Then the swarm result should not reject workflow

  Scenario: A workflow-enabled agent cannot create a swarm
    When a workflow-enabled agent tries to create a swarm run
    Then the swarm result should be an error
    And the swarm result should contain "cannot create a swarm"

  Scenario: A superseding message replaces its predecessor in the inbox
    When a swarm member sends a message and then supersedes it with a newer revision
    Then the recipient inbox holds only the newer message with its revision
    And the superseded message remains in the audit

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

  Scenario: Resolved approval keeps the original claim and reservations
    When an owned blocked swarm task is unblocked
    Then the resumed swarm task retains its claim and reserved file

  Scenario: Durable pause retains the board and allows explicit resume
    When a swarm member creates an acceptance task
    And the supervisor durably pauses the swarm
    Then the swarm run status is "paused"
    And the swarm summary retains one task
    When the supervisor resumes the swarm
    Then the swarm run status is "running"
    And a later swarm execution sees the acceptance task

  Scenario: A coordinator stop ends the run as a pause only the supervisor resumes
    When the coordinator stops the run as "blocked"
    Then the swarm run is paused holding "blocked"
    And every swarm member is still live
    When a swarm member tries to resume the run
    Then the swarm result should contain "outside the swarm"
    When the supervisor outside the swarm resumes the run
    Then the swarm run status is "running"
    And the coordinator can run Python on the board again

  Scenario: Completion holds success until the supervisor closes it
    When the coordinator completes the run with accepted evidence
    Then the swarm run is paused holding "succeeded"
    When the supervisor outside the swarm closes the run
    Then the swarm run status is "succeeded"
    And a swarm member can no longer create work

  Scenario: Deadline expiry pauses the run until the supervisor grants more time
    When the swarm deadline has passed
    Then the swarm run is paused holding "budget-exhausted"
    When the supervisor outside the swarm resumes the run
    Then the swarm result should contain "extend the deadline"
    When the supervisor outside the swarm extends the deadline by 600 seconds
    And the supervisor outside the swarm resumes the run
    Then the swarm run status is "running"

  Scenario: Repeated paused inspection returns a compact delta
    When the supervisor durably pauses the swarm
    And the supervisor inspects the unchanged swarm cursor
    Then the swarm inspection is unchanged without task history

  @done @swarm-supervision
  Scenario: A resume restores a member suspended by a provider failure
    When a provider failure suspends the coordinator and the parent resumes the run
    Then the coordinator is re-armed by the resume alone and continues its work

  @done @swarm-supervision
  Scenario: Supervisor pauses active work and receives approval handling and retained evidence
    When the supervisor pauses active work then delivers approval and exports evidence
    Then the swarm approval has a completed receipt and a retained terminal report
    And the observed usage budget pauses the run and request accounting is available

  Scenario: The coordinator revokes a claim its suspended owner will not finish
    When a suspended member holds a claimed task with a reserved file
    And the coordinator revokes that claim as "member suspended by provider"
    Then the revoked task is ready without owner, reservation or evidence
    And the revocation is audited with its reason and previous owner
    And the previous owner is told its claim was revoked
    When another member claims and completes the revoked task
    Then the swarm task status is "completed"
    And the revoked owner's stale token can no longer act on the task

  Scenario: Only the coordinator can revoke a claim
    When a suspended member holds a claimed task with a reserved file
    And a member other than the coordinator tries to revoke that claim
    Then the swarm result should be an error
    And the swarm result should contain "only the designated coordinator"

  Scenario: A task row shows its quiet owner's liveness and names it as a send recipient
    When a suspended member holds a claimed task with a reserved file
    And that owner has been silent on the board for 400 seconds
    Then the task row reads the owner as idle for at least 400 seconds with a send contact
    And the swarm tool description says any member may message a task's owner

  @done @swarm-supervision
  Scenario: A member the coordinator kills is confirmed dead and its work recovered without pausing the run
    When a member the coordinator launched is killed while holding a claim and another member reconciles throughout
    Then the run kept running, the member is dead and another member takes over its recovered work

  @done @swarm-supervision
  Scenario: An unobserved member loss is recorded once and never re-pauses a resumed run
    When an unobserved member loss pauses the run and the supervisor resumes it
    Then the stale loss never pauses the run again and the work is revoked and reclaimed
