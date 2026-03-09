@done
Feature: End-to-End Provider Wiring
  As a user running the agent CLI
  I want provider selection, fallback, and credential resolution to work end-to-end
  So that the agent reliably reaches an LLM even when one provider is down

  # --- Provider selection ---

  Scenario: Agent uses OpenAI provider when configured
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "Hello from OpenAI"
    When I run quecto agent -s - -m "Hi"
    Then the exit code should be 0
    And stdout should contain "Hello from OpenAI"

  Scenario: Agent uses Anthropic provider when configured
    Given a temp base directory
    And a config file with an Anthropic provider pointing at a mock server
    And the Anthropic mock returns a text response "Hello from Anthropic"
    When I run quecto agent -s - -m "Hi"
    Then the exit code should be 0
    And stdout should contain "Hello from Anthropic"

  # --- No silent fallback (ProviderRouter) ---

  Scenario: Agent fails immediately on first-provider error (no silent fallback)
    Given a temp base directory
    And a config file with both OpenAI and Anthropic providers pointing at mock servers
    And the OpenAI mock returns an HTTP 500 error
    And the Anthropic mock returns a text response "Fallback worked"
    When I run quecto agent -s - -m "Hi"
    Then the exit code should be 1
    And stderr should contain "Error"

  Scenario: Agent fails when all providers return errors
    Given a temp base directory
    And a config file with both OpenAI and Anthropic providers pointing at mock servers
    And the OpenAI mock returns an HTTP 500 error
    And the Anthropic mock returns an HTTP 500 error
    When I run quecto agent -s - -m "Hi"
    Then the exit code should be 1
    And stderr should contain "Error"

  # --- Credential store integration ---

  Scenario: Agent uses credential store token over config file key
    Given a temp base directory
    And a config file with OpenAI api_key "sk-from-config" pointing at a mock server
    And the credential store has a valid token "sk-from-store" for provider "openai"
    And the mock expects Authorization header "Bearer sk-from-store" and returns "Authenticated"
    When I run quecto agent -s - -m "Hi"
    Then the exit code should be 0
    And stdout should contain "Authenticated"

  Scenario: Agent falls back to config key when credential is expired
    Given a temp base directory
    And a config file with OpenAI api_key "sk-from-config" pointing at a mock server
    And the credential store has an expired token "sk-expired" for provider "openai"
    And the mock expects Authorization header "Bearer sk-from-config" and returns "Config key used"
    When I run quecto agent -s - -m "Hi"
    Then the exit code should be 0
    And stdout should contain "Config key used"

  # --- Auth errors ---

  Scenario: Auth error from provider is not retried on same provider
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the OpenAI mock returns an HTTP 401 error
    When I run quecto agent -s - -m "Hi"
    Then the exit code should be 1
    And stderr should contain "Error"
