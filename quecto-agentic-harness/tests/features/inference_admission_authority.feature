@inference-admission @done
Feature: Shared admission authority across sessions
  A same-user authority process bounds outbound attempts across independent
  roots, keeps abandoned work as uncertain occupancy and recovers only through
  verified completion or an explicit operator reset.

  Scenario: Two independent roots share one capacity slot
    Given a running admission authority with capacity one
    And two independent root sessions bound to the authority
    When both roots request inference through the same alias
    Then exactly one attempt is active and the other is queued
    When the active root completes its attempt
    Then the queued root is granted

  Scenario: A vanished session quarantines its group until its capability reconciles
    Given a running admission authority with capacity two
    And a root session holding an active attempt
    When the holding session's connection is lost
    Then the authority reports one uncertain attempt
    And another root's request is refused without a grant
    When the same capability reconnects and completes the attempt
    Then the authority reports no uncertain attempts and grants again

  Scenario: An operator reset revokes capabilities and starts a new epoch
    Given a running admission authority with capacity two
    And a root session holding an active attempt
    When the holding session's connection is lost
    And the operator resets the authority
    Then the epoch advances and the old capability is unauthorized
    And a fresh root session is granted in the new epoch
