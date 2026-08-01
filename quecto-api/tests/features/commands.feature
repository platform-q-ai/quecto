@done @issue-1139
Feature: Agent control endpoints
  As a client application
  I want to steer, follow up, abort, switch model, and inspect subagents/tools
  So that I can drive the full UDS capability model over HTTP

  Scenario: Steer the running agent
    Given the agent is connected
    When I POST /steer with body:
      """
      {"message": "focus on tests"}
      """
    Then the response status is 200
    And the response body contains "type":"response"
    And the response body contains "messageEcho":"focus on tests"

  Scenario: Steer with empty message is rejected
    Given the agent is connected
    When I POST /steer with body:
      """
      {"message": ""}
      """
    Then the response status is 400
    And the response body contains "invalid request"

  Scenario: Steer when agent disconnected
    Given the agent is not connected
    When I POST /steer with body:
      """
      {"message": "hi"}
      """
    Then the response status is 503

  Scenario: Follow up on the agent
    Given the agent is connected
    When I POST /follow_up with body:
      """
      {"message": "then summarize"}
      """
    Then the response status is 200
    And the response body contains "messageEcho":"then summarize"

  Scenario: Abort the current run
    Given the agent is connected
    When I POST /abort with an empty body
    Then the response status is 200
    And the response body contains "type":"response"

  Scenario: Abort when agent disconnected
    Given the agent is not connected
    When I POST /abort with an empty body
    Then the response status is 503

  Scenario: Switch the active model
    Given the agent is connected
    When I POST /model with body:
      """
      {"model": "anthropic/claude"}
      """
    Then the response status is 200
    And the response body contains "success":true

  Scenario: Switch model with no fields is rejected
    Given the agent is connected
    When I POST /model with an empty body
    Then the response status is 400
    And the response body contains "invalid request"

  Scenario: Set the reasoning effort
    Given the agent is connected
    When I POST /effort with body:
      """
      {"effort": "high"}
      """
    Then the response status is 200
    And the response body contains "effortEcho":"high"

  Scenario: Set effort normalizes case and whitespace
    Given the agent is connected
    When I POST /effort with body:
      """
      {"effort": "  MEDIUM "}
      """
    Then the response status is 200
    And the response body contains "effortEcho":"medium"

  Scenario: Set effort with an unknown level is rejected
    Given the agent is connected
    When I POST /effort with body:
      """
      {"effort": "turbo"}
      """
    Then the response status is 400
    And the response body contains "invalid request"

  Scenario: Set effort when agent disconnected
    Given the agent is not connected
    When I POST /effort with body:
      """
      {"effort": "low"}
      """
    Then the response status is 503

  Scenario: Clear conversation history
    Given the agent is connected
    When I POST /clear_history with an empty body
    Then the response status is 200
    And the response body contains "type":"response"

  Scenario: Clear history when agent disconnected
    Given the agent is not connected
    When I POST /clear_history with an empty body
    Then the response status is 503

  Scenario: List subagents
    Given the agent is connected
    When I request GET /subagents
    Then the response status is 200
    And the response body contains "success":true

  Scenario: List tools through the rich catalogue
    Given the agent is connected
    When I request GET /tools/catalogue
    Then the response status is 200
    And the response body contains "success":true

  Scenario: List tools through the short alias
    Given the agent is connected
    When I request GET /tools
    Then the response status is 200
    And the response body contains "success":true
