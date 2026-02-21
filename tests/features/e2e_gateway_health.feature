@done
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
    When I start the quecto gateway subprocess
    And I wait for the health server to accept connections
    And I request GET "/health" from the gateway health server
    Then the HTTP response status should be 200
    And the response body should contain "ok"

  # --- Readiness ---

  Scenario: Gateway /ready returns 200 when provider is available
    Given the config has health server enabled on a random port
    When I start the quecto gateway subprocess
    And I wait for the health server to accept connections
    And I request GET "/ready" from the gateway health server
    Then the HTTP response status should be 200
    And the response body should contain "true"

  # --- Health server alongside Telegram polling ---

  Scenario: Health server runs concurrently with Telegram message processing
    Given the config has health server enabled on a random port
    And a mock Telegram API with one pending update from user "12345" with text "Hello"
    And the mock LLM returns a text response "Hi there"
    When I start the quecto gateway subprocess
    And I wait for the health server to accept connections
    And I request GET "/health" from the gateway health server
    Then the HTTP response status should be 200
