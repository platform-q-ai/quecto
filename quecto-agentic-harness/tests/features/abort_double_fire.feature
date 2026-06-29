@pending
Feature: Abort command does not double-fire cancellation
  Issue #512: The reader task fires cancel eagerly, then handle_abort
  fires it again. The second fire pre-cancels the NEXT prompt.

  Scenario: Abort followed by new prompt executes normally
    Given the agent is running a prompt
    When the client sends an abort command
    And the agent acknowledges the abort
    And the client sends a new prompt
    Then the new prompt should be processed (not silently skipped)

  Scenario: Cancel slot returns to Idle after abort
    Given the agent is running and the cancel slot is Armed
    When abort fires the cancel (reader task)
    Then the slot should be Idle (not Fired)
    And arm_cancel on the next prompt should return Some(rx)
