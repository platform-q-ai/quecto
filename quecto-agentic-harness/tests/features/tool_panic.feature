@done @tool-panic
Feature: A panicking tool costs one call, never the agent

  A bug in one tool must not end the agent that called it: the call answers
  an internal error, the event log records where it panicked, and the turn
  goes on. A panic anywhere else still ends the agent at once, leaving a
  record of why it died (#2192).

  Scenario: A tool that panics costs its call and the agent keeps going
    Given a tool panic workspace
    When a one-shot agent calls a tool that panics
    Then the agent finished its answer after the failed call
    And the event log records the panic with the tool and its location

  Scenario: A panic outside any tool call ends the agent
    Given a tool panic workspace
    When a one-shot agent calls a tool whose side thread panics
    Then the agent was aborted
    And its crash record and event log say why it died

  Scenario: A parent reads why a crashed sub-agent ended and what it did
    Given a tool panic workspace
    When a parent agent's sub-agent survives one tool panic and then crashes
    Then the parent is told the sub-agent ended unexpectedly while its tool call ran, with the panic
    And the parent's get_state names the end reason
    And the parent's get_messages returns the sub-agent's transcript up to the crash
