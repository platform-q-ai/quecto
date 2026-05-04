@wip
Feature: Prompt endpoint
  As a client application
  I want to send prompts to the quecto agent via HTTP
  So that I can integrate AI capabilities into my app

  Scenario: Send a prompt and receive a response
    Given the agent is connected
    When I POST /prompt with body:
      """
      {"message": "Hello, what can you do?"}
      """
    Then the response status is 200
    And the response body contains "type":"response"
    And the response body contains "success":true

  Scenario: Send a long-running prompt without waiting for completion
    Given the agent is connected
    When I POST /prompt with body:
      """
      {"message": "Run a long workflow", "waitForCompletion": false}
      """
    Then the response status is 200
    And the response body contains "accepted":true

  Scenario: Prompt when agent is disconnected
    Given the agent is not connected
    When I POST /prompt with body:
      """
      {"message": "Hello"}
      """
    Then the response status is 503
    And the response body contains "error"

  Scenario: Prompt with empty message is rejected
    Given the agent is connected
    When I POST /prompt with body:
      """
      {"message": ""}
      """
    Then the response status is 400
    And the response body contains "invalid request"
