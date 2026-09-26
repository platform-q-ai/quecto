@done @event-log
Feature: Switchable event log

  With telemetry.event_log switched on, every agent keeps a detailed,
  private log of its model requests, turns and tool calls, whatever mode it
  runs in; one asked to leave nothing behind keeps none (#2150).

  Scenario: A UDS agent with the event log on logs its tool calls
    Given an event log workspace with the event log switched on
    When a UDS agent answers a prompt that runs a tool
    Then the event log holds the tool result with its duration and sizes

  Scenario: A one-shot agent with the event log on logs its tool calls
    Given an event log workspace with the event log switched on
    When a one-shot agent answers a prompt that runs a tool
    Then the event log holds the tool result with its duration and sizes

  Scenario: An agent asked to leave nothing behind keeps no event log
    Given an event log workspace with the event log switched on
    When an ephemeral one-shot agent answers a prompt that runs a tool
    Then no event log was written

  Scenario: With the event log off an ordinary agent keeps none
    Given an event log workspace with the event log switched off
    When a UDS agent answers a prompt that runs a tool
    Then no event log was written
