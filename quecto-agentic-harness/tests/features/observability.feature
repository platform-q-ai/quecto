@done
Feature: Observability
  As an operator
  I want structured logging
  So that I can monitor Quecto in production

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
    When the agent processes a [message] that triggers a tool call
    Then the captured log output should include span "tool_exec"
    And the captured log output should include field "tool_name"
    And the captured log output should include field "duration_ms"

  Scenario: API keys are redacted in log output
    Given a tracing subscriber capturing JSON log output
    When the [message] "Provider configured with key sk-secret-key-12345" is logged at info level
    Then the captured log output should not contain "sk-secret-key-12345"
    And the captured log output should contain a redacted placeholder
