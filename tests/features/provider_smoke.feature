Feature: Provider Smoke
  Minimal live provider checks. These scenarios are excluded unless
  QUECTO_PROVIDER_SMOKE=1 is set.

  @done @provider-smoke @provider-smoke-openai
  Scenario: OpenAI accepts a minimum-output agent request
    Given an OpenAI provider smoke workspace is configured
    When I run the OpenAI provider smoke agent with message "Reply exactly OK"
    Then the exit code should be 0
    And stdout should not be empty

  @done @provider-smoke @provider-smoke-anthropic
  Scenario: Anthropic accepts a minimum-output agent request
    Given an Anthropic provider smoke workspace is configured
    When I run the Anthropic provider smoke agent with message "Reply exactly OK"
    Then the exit code should be 0
    And stdout should not be empty

  @done @provider-smoke @provider-smoke-codex
  Scenario: Codex accepts a minimum-output agent request
    Given a Codex provider smoke workspace is configured
    When I run the Codex provider smoke agent with message "Reply exactly OK"
    Then the exit code should be 0
    And stdout should not be empty
