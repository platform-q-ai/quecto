@inference-admission @wip
Feature: Leaf provider attempts share admission without gating delegation
  Admission remains disabled in production until shared authority verification.

  Scenario: A streaming receiver is not a completed transport
    Given a phase-local provider admission group with one slot
    And a fake provider response held open behind a transport barrier
    When an incremental inference receiver is returned
    And another inference attempt queues in the same group
    Then the second attempt has not started HTTP
    When the first owned transport acknowledges completion
    Then the second attempt starts exactly one HTTP request

  Scenario: Valid header advice blocks siblings before the error body completes
    Given a phase-local provider admission group with two slots
    And a fake throttle response with a valid ninety second Retry-After
    When the throttle headers arrive while its body remains blocked
    Then a sibling cannot start before the ninety second deadline
    And an independent quota group can still start inference

  Scenario: Admission never replays partially emitted output
    Given an admitted fake provider stream that has emitted text
    When a typed overload event ends the stream
    Then shared throttle feedback is recorded
    And admission does not issue another HTTP attempt
