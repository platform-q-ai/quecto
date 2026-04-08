@wip
Feature: WebSocket event stream
  As a client application
  I want to subscribe to agent events via WebSocket
  So that I can show streaming tokens and tool executions in real time

  Scenario: Connect and receive agent events
    Given the agent is connected
    When I connect a WebSocket to /ws
    And I send a prompt "What is 2+2?" via the WebSocket
    Then I receive an agent_start event
    And I receive one or more token events
    And I receive an agent_end event
    And the WebSocket remains open

  Scenario: WebSocket disconnects gracefully when agent stops
    Given the agent is connected
    And I have an open WebSocket to /ws
    When the agent disconnects
    Then the WebSocket is closed with a normal close code

  Scenario: Multiple WebSocket clients receive the same events
    Given the agent is connected
    And I have 2 open WebSocket connections to /ws
    When I send a prompt "Hello" via WebSocket client 1
    Then both WebSocket clients receive the agent_start event
    And both WebSocket clients receive the agent_end event
