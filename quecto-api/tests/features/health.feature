@wip
Feature: Health endpoint
  As an operator
  I want a health check endpoint
  So that Kubernetes can determine pod readiness

  Scenario: Healthy when agent is connected
    Given the agent is connected
    When I request GET /health
    Then the response status is 200
    And the response body contains "healthy":true
    And the response body contains "agent_connected":true

  Scenario: Unhealthy when agent is disconnected
    Given the agent is not connected
    When I request GET /health
    Then the response status is 503
    And the response body contains "healthy":false
    And the response body contains "agent_connected":false
