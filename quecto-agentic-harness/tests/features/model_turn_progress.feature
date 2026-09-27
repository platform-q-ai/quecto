@done @issue-2210
Feature: Supervising a model turn in flight

  While a child's model thinks or streams, a supervising parent sees how
  long the turn has run, what its attempt has streamed and how long since
  the provider last sent anything, so it can tell a live but runaway reply
  from a hung one. A runaway reply is stopped at its output cap, and a turn
  stopped where it stands leaves a record of the request it interrupted.

  Scenario: A busy agent's state shows the model turn in flight
    Given a model turn workspace whose provider streams two deltas and then stalls
    When a UDS agent is prompted and its state is requested while the reply streams
    Then the state shows the model turn with its attempt's events, output and last event

  Scenario: A turn stopped by a termination signal records the request it interrupted
    Given a model turn workspace whose provider streams two deltas and then stalls
    When a UDS agent is prompted and stopped by a termination signal while the reply streams
    Then the event log holds the request as cancelled with its attempt interrupted after its output

  Scenario: A one-shot run stopped at its deadline records the request it interrupted
    Given a model turn workspace whose provider streams two deltas and then stalls
    When a one-shot agent is prompted with a 3 second run deadline
    Then the event log holds the request as cancelled with its attempt interrupted

  Scenario: A runaway reply is stopped at its output cap and not retried
    Given a model turn workspace whose provider streams without end
    When a UDS agent is prompted until its turn ends
    Then the turn ends with the output cap error
    And the provider was asked once
    And the event log holds the attempt stopped at its output cap
