@done
Feature: Observability
  As an operator
  I want structured logging and health endpoints
  So that I can monitor Quecto in production

  @pending
  Scenario: Health endpoint returns OK
    Given a running gateway
    When I request GET "/health"
    Then the response status should be 200
    And the body should contain "ok"

  @pending
  Scenario: Ready endpoint returns OK
    Given a running gateway
    When I request GET "/ready"
    Then the response status should be 200

  Scenario: Status command shows configuration summary
    Given a valid config with OpenAI API key set
    When I run quecto with arguments "status"
    Then the output should contain "quecto Status"
    And the output should contain "Config:"
    And the output should contain "Workspace:"
    And the output should contain "Model:"

  Scenario: Status shows provider availability
    Given a config with OpenAI api_key set and Anthropic not set
    When I run quecto with arguments "status"
    Then the output should contain "OpenAI API:"
    And the output should contain "configured"
    And the output should contain "Anthropic API:"
    And the output should contain "not set"

  Scenario: Status output redacts API keys
    Given a config with OpenAI api_key "sk-secret-key-12345" set
    When I run quecto with arguments "status"
    Then the output should not contain "sk-secret-key-12345"
    And the output should contain "OpenAI API:"

  @pending
  Scenario: Structured logging includes category and fields
    Given a running gateway with debug logging enabled
    When a tool execution occurs
    Then the log output should include a "tool" category
    And the log output should include "duration_ms"

  @pending
  Scenario: Logs do not expose API keys in log output
    Given a config with OpenAI api_key "sk-secret-key"
    When the agent initializes with debug logging
    Then the log output should not contain "sk-secret-key"
