@inference-admission @done
Feature: Admission activity is projected to socket clients beside the lifecycle
  A supervisor polling get_state sees a queued attempt as waiting with its
  group and wait, never as an idle or stalled process; the admission view
  advances the state cursor exactly like any other component and every
  transition is pushed as admission_state_changed.

  Scenario: A queued attempt is reported as waiting, never as quiet or a stall
    Given a running admission authority with capacity one
    And a root session holding an active attempt
    When an observed process attempt queues behind the holder
    And a supervisor polls the process state
    Then the state is "thinking" with progress "waiting" naming group "g"
    And the admission view shows 1 waiting and 0 admitted
    When the holding root completes its attempt
    Then the process observes the attempt admitted and then completed
    When a supervisor polls the process state
    Then the state is "thinking" with progress "active" naming group ""
    And the admission view shows 0 waiting and 0 admitted

  Scenario: The since cursor stays unchanged until an admission transition
    Given a running admission authority with capacity one
    And a root session holding an active attempt
    When an observed process attempt queues behind the holder
    And a supervisor polls the process state
    And the supervisor polls again with the generation it last saw
    Then the response is the unchanged marker
    When the holding root completes its attempt
    Then the process observes the attempt admitted and then completed
    When the supervisor polls again with the generation it last saw
    Then the response is a changed view with progress "active"
    When the supervisor polls again with the generation it last saw
    Then the response is the unchanged marker

  Scenario: Aborting a run while its attempt waits cancels the wait
    Given a running admission authority with capacity one
    And a root session holding an active attempt
    When an observed process attempt queues behind the holder
    And a supervisor polls the process state
    Then the state is "thinking" with progress "waiting" naming group "g"
    When the waiting run is aborted
    And a supervisor polls the process state
    Then the state is "idle" with progress "quiet" naming group ""
    And the admission view counts 1 cancelled attempt and nothing waiting

  Scenario: Every admission transition is pushed to socket clients
    Given a running admission authority with capacity one
    And a root session holding an active attempt
    When an observed process attempt queues behind the holder
    And the process pushes admission transitions to its socket clients
    And the holding root completes its attempt
    Then the process observes the attempt admitted and then completed
    And the clients receive admission_state_changed events ending with 1 completed
