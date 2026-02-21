@done
Feature: Observability
  As an operator
  I want structured logging and health endpoints
  So that I can monitor Quecto in production

  @pending
  Scenario: Health endpoint returns OK when gateway is running
    Given a running gateway with health server on port 9090
    When I request GET "http://127.0.0.1:9090/health"
    Then the response status should be 200
    And the response body should be JSON containing "status" with value "ok"

  @pending
  Scenario: Ready endpoint checks provider availability
    Given a running gateway with health server on port 9090
    And at least one LLM provider is configured
    When I request GET "http://127.0.0.1:9090/ready"
    Then the response status should be 200
    And the response body should be JSON containing "ready" with value true

  @pending
  Scenario: Ready endpoint returns 503 when no providers available
    Given a running gateway with health server on port 9090
    And no LLM providers are configured
    When I request GET "http://127.0.0.1:9090/ready"
    Then the response status should be 503
    And the response body should be JSON containing "ready" with value false

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
  Scenario: Structured logging includes span fields for tool execution
    Given a running gateway with RUST_LOG set to "debug"
    And a mock LLM provider
    When the agent executes a tool call
    Then the log output should include a "tool_exec" span
    And the log output should include field "tool_name"
    And the log output should include field "duration_ms"

  @pending
  Scenario: API keys are redacted in all log output
    Given a config with OpenAI api_key "sk-secret-key-12345"
    And RUST_LOG set to "trace"
    When the gateway starts and initializes providers
    Then the log output should not contain "sk-secret-key-12345"
    And the log output should contain "sk-***" or a redacted placeholder
