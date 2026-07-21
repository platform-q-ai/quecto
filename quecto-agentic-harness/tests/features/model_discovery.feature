Feature: Runtime model discovery
  As a Quecto user
  I want provider catalogs to update my runtime model registry
  So that changing OpenAI-compatible model lists are available without kernel-side discovery

  @done
  Scenario: Discovery updates only the selected provider catalog
    Given provider "openrouter" has auth, custom settings, and an old model
    And provider "anthropic-api" has its own auth and models
    And the OpenAI-compatible catalog for "openrouter" returns models "alpha" and "beta"
    When I discover models for provider "openrouter"
    Then the "openrouter" catalog should contain models "alpha" and "beta"
    And the "openrouter" auth and custom settings should be unchanged
    And the "anthropic-api" provider should be unchanged

  @done
  Scenario: Discovery publishes a complete registry atomically
    Given provider "local-openai" has an empty model catalog
    And the OpenAI-compatible catalog for "local-openai" returns model "local"
    When I discover models for provider "local-openai"
    Then the models registry should remain valid JSON
    And no discovery temporary file should remain
