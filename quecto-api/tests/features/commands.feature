@done @issue-1139
Feature: Agent control endpoints
  As a client application
  I want to steer, follow up, abort, switch model, and inspect subagents/extensions
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

  Scenario: List subagents
    Given the agent is connected
    When I request GET /subagents
    Then the response status is 200
    And the response body contains "success":true

  Scenario: List extensions
    Given the agent is connected
    When I request GET /extensions
    Then the response status is 200
    And the response body contains "success":true

  Scenario: Reload extensions
    Given the agent is connected
    When I POST /extensions/reload with an empty body
    Then the response status is 200
    And the response body contains "success":true
