@wip
Feature: State and history endpoints
  As a client application
  I want to query agent state, messages, and session stats
  So that I can display conversation history and agent status

  Scenario: Get agent state
    Given the agent is connected
    When I request GET /state
    Then the response status is 200
    And the response body contains "success":true

  Scenario: Get message history
    Given the agent is connected
    When I request GET /messages
    Then the response status is 200
    And the response body contains "success":true

  Scenario: Get tail of messages
    Given the agent is connected
    When I request GET /messages/tail?n=5
    Then the response status is 200
    And the response body contains "success":true

  Scenario: Get session stats
    Given the agent is connected
    When I request GET /stats
    Then the response status is 200
    And the response body contains "success":true

  Scenario: State endpoints return 503 when disconnected
    Given the agent is not connected
    When I request GET /state
    Then the response status is 503
