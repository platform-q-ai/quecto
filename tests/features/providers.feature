@done
Feature: LLM Providers
  As a user
  I want to use OpenAI or Anthropic as my LLM provider
  So that I can choose the best model for my needs

  Scenario: Select OpenAI provider via config
    Given a config with provider "openai" and api_key "sk-test"
    When I create a provider from config
    Then the provider should be "openai"

  Scenario: Select Anthropic provider via config
    Given a config with provider "anthropic" and api_key "sk-ant-test"
    When I create a provider from config
    Then the provider should be "anthropic"

  Scenario: Error classification distinguishes retryable errors
    Given a provider error with status 429
    Then the error should be classified as "rate_limit"
    And the error should be retryable

  Scenario: Error classification for auth errors
    Given a provider error with status 401
    Then the error should be classified as "auth"
    And the error should not be retryable

  Scenario: Error classification for server errors
    Given a provider error with status 500
    Then the error should be classified as "server"
    And the error should be retryable

  Scenario: Provider fallback on server error
    Given a primary provider that returns a server error "HTTP 500 Internal Server Error"
    And a fallback provider that returns "Fallback response"
    When I send a chat request through the fallback provider
    Then the fallback response content should be "Fallback response"

  Scenario: Provider respects cooldown after rate limit
    Given a primary provider that returns a rate limit error "HTTP 429 rate limit"
    And a fallback provider that returns "Cooldown fallback"
    When I send a chat request through the fallback provider
    And I send a second chat request through the fallback provider
    Then the fallback response content should be "Cooldown fallback"

  Scenario: Provider sends chat request with tools
    Given an OpenAI provider with a mock server
    And the mock server returns a chat response with content "Hello!"
    When I send a chat request with message "Hi" and a tool "exec"
    Then the chat response content should be "Hello!"
    And the chat request should have included an Authorization header

  Scenario: OpenAI provider handles streaming responses
    Given an OpenAI provider with a mock server
    And the mock server returns an OpenAI streaming response with content "Hello world"
    When I send a streaming chat request with message "Hi"
    Then the streaming response content should be "Hello world"

  Scenario: Anthropic provider handles streaming responses
    Given an Anthropic provider with a mock server
    And the mock server returns an Anthropic streaming response with content "Hello from Claude"
    When I send a streaming chat request with message "Hi"
    Then the streaming response content should be "Hello from Claude"
