@inference-admission @done
Feature: Admission activity is observable without being a lifecycle state
  A process can see whether its attempts are waiting for admission, admitted,
  or refused, with the quota group and elapsed wait, as a bounded and fresh
  view that never masquerades as idle or stalled.

  Scenario: A queued attempt is visible as waiting with its group and elapsed wait
    Given a running admission authority with capacity one
    And a root session holding an active attempt
    When an observed process attempt queues behind the holder
    Then the process observes one waiting attempt in group "g" with a growing wait
    When the holding root completes its attempt
    Then the process observes the attempt admitted and then completed
