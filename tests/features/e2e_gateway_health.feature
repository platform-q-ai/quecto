@pending
Feature: End-to-End Gateway Health Server
  As a system operator
  I want the gateway to expose health and readiness endpoints
  So that load balancers and monitoring systems can check if Quecto is alive

  The gateway should start a health HTTP server as part of its event loop.
  The /health endpoint always returns 200 (liveness probe). The /ready
  endpoint returns 200 when at least one LLM provider is available, and
  503 otherwise (readiness probe). These tests verify the gateway actually
  starts the health server, not just that HealthServer works in isolation.

  Background:
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server

  # --- Liveness ---

  Scenario: Gateway exposes /health endpoint that returns 200
    Given the config has health server enabled on a random port
    When I start the quecto gateway
    And I wait for the gateway to be ready
    And I request GET "/health" from the gateway health server
    Then the HTTP response status should be 200
    And the response body should contain "ok"

  # --- Readiness ---

  Scenario: Gateway /ready returns 200 when provider is available
    Given the config has health server enabled on a random port
    And the mock LLM returns a text response "healthy"
    When I start the quecto gateway
    And I wait for the gateway to be ready
    And I request GET "/ready" from the gateway health server
    Then the HTTP response status should be 200
    And the response body should contain "true"

  Scenario: Gateway /ready returns 503 when no providers are available
    Given the config has health server enabled on a random port
    And all provider API keys are removed from config
    When I start the quecto gateway
    And I wait for the gateway to be ready
    And I request GET "/ready" from the gateway health server
    Then the HTTP response status should be 503
    And the response body should contain "false"

  # --- Health server disabled ---

  Scenario: Gateway does not expose health endpoints when disabled
    Given the config has health server disabled
    When I start the quecto gateway
    And I wait 2 seconds for the gateway to stabilize
    Then no HTTP server should be listening on the health port

  # --- Health server alongside Telegram polling ---

  Scenario: Health server runs concurrently with Telegram message processing
    Given the config has health server enabled on a random port
    And a mock Telegram API with one pending message "Hello"
    And the mock LLM returns a text response "Hi there"
    When I start the quecto gateway
    And I wait for the gateway to be ready
    And I request GET "/health" from the gateway health server
    Then the HTTP response status should be 200
    And the Telegram API should eventually receive a sendMessage
