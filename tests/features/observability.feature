@done
Feature: Observability
  As an operator
  I want structured logging and health endpoints
  So that I can monitor Quecto in production

  Scenario: Health endpoint returns OK
    Given a health server started on a random port
    When I request GET "/health" from the health server
    Then the HTTP response status should be 200
    And the response body should be JSON containing "status" with value "ok"

  Scenario: Ready endpoint reports ready when providers are available
    Given a health server started on a random port
    And the readiness check reports providers available
    When I request GET "/ready" from the health server
    Then the HTTP response status should be 200
    And the response body should be JSON containing "ready" with value "true"

  Scenario: Ready endpoint returns 503 when no providers available
    Given a health server started on a random port
    And the readiness check reports no providers available
    When I request GET "/ready" from the health server
    Then the HTTP response status should be 503
    And the response body should be JSON containing "ready" with value "false"

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

  Scenario: Structured logging includes span fields for tool execution
    Given an agent loop with a mock provider and mock tools
    And a tracing subscriber capturing JSON log output
    When the agent processes a message that triggers a tool call
    Then the captured log output should include span "tool_exec"
    And the captured log output should include field "tool_name"
    And the captured log output should include field "duration_ms"

  Scenario: API keys are redacted in log output
    Given a tracing subscriber capturing JSON log output
    When the message "Provider configured with key sk-secret-key-12345" is logged at info level
    Then the captured log output should not contain "sk-secret-key-12345"
    And the captured log output should contain a redacted placeholder
