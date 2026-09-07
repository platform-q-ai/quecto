@inference-admission @done
Feature: Deterministic inference admission policy
  Policy tests do not enable provider or cross-process enforcement.

  Scenario: An idle parent does not consume its child's inference capacity
    Given an admission group with one slot and an idle interactive parent
    When its background child queues an inference attempt
    And admission is dispatched at time zero
    Then the child is admitted without waiting for its parent

  Scenario: Cancelling queued inference prevents later dispatch
    Given an admission group with one slot and an idle interactive parent
    When its background child queues an inference attempt
    And the queued child inference is cancelled
    And admission is dispatched after cancellation
    Then no child inference is dispatched

  Scenario: Confirming attempt completion does not refund request pacing
    Given an admission group with one slot and an idle interactive parent
    And admission requests are paced ten milliseconds apart
    When its background child completes one attempt and queues another
    And admission is attempted just before and at the pacing boundary
    Then the next attempt waits until the configured pacing boundary
