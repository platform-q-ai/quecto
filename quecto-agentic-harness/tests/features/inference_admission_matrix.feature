@inference-admission @done
Feature: Integrated admission matrix over a real authority (AC2, AC3, AC6)
  The fairness, pacing and failure-safety rules proven at the policy level
  hold end to end through the authority's socket protocol and the process
  client: adding descendants cannot increase a root's share, background
  work leaves the interactive reserve free, bounded queues refuse and time
  out explicitly, an authority outage fails closed, and pacing applies to
  every start.

  Scenario: Round-robin across roots means a descendant cannot increase its root's share
    Given a running admission authority with capacity one
    And two independent root sessions bound to the authority
    And the first root has registered a bound child
    When the first root holds the slot while its child and then the second root queue
    And the first root completes its attempt
    Then the second root is granted before the first root's child
    And the child is granted once the second root completes

  Scenario: Background attempts leave the interactive reserve free
    Given a running admission authority with capacity two and an interactive reserve of one
    When two background roots request inference
    Then only one background attempt is active and the other waits
    When an interactive root requests inference
    Then the interactive root is granted immediately alongside the background attempt

  Scenario: A full queue refuses explicitly and a queued wait times out explicitly
    Given a running admission authority with capacity one, a queue of one and a short queue deadline
    And a root session holding an active attempt
    When a second root queues and a third root requests
    Then the third root is refused as queue full without a grant
    And the second root's wait ends with an explicit deadline error and no grant

  Scenario: An authority outage fails closed instead of falling back to unbounded requests
    Given a running admission authority with capacity one
    And two independent root sessions bound to the authority
    When the authority stops
    Then a root's next attempt fails explicitly within the bounded wait

  Scenario: Pacing applies to every start across independent roots
    Given a running admission authority with capacity two and a start interval of 200 milliseconds
    And two independent root sessions bound to the authority
    When both roots request inference back to back
    Then the second grant starts no earlier than the pacing interval after the first
