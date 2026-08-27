Feature: Runtime model discovery
  As a Quecto user
  I want provider catalogs to refresh into the discovered-layer cache
  So that changing OpenAI-compatible model lists reach the effective catalogue without rewriting my models.json

  @done
  Scenario: Discovery caches only the selected provider's models
    Given provider "openrouter" has auth, custom settings, and an old model
    And provider "anthropic-api" has its own auth and models
    And the OpenAI-compatible catalog for "openrouter" returns models "alpha" and "beta"
    When I discover models for provider "openrouter"
    Then the "openrouter" discovery cache should contain models "alpha" and "beta"
    And the user-owned models registry should be unchanged by discovery
    And no discovery cache should exist for provider "anthropic-api"

  @done
  Scenario: Discovery publishes a complete cache atomically
    Given provider "local-openai" has an empty model catalog
    And the OpenAI-compatible catalog for "local-openai" returns model "local"
    When I discover models for provider "local-openai"
    Then the "local-openai" discovery cache should be valid JSON
    And no discovery temporary file should remain
