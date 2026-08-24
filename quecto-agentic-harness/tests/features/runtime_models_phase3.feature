@done @runtime-models-phase3
Feature: Runtime model registry for discoverable models
  As a long-running UDS agent
  I want built-in and configured models to be loaded at runtime
  So that first-party and community models are discoverable and usable without restart

  # Acceptance criteria for #956:
  # - Claude Sonnet 5 is discoverable as both anthropic-api/claude-sonnet-5 and anthropic-oauth/claude-sonnet-5.

  Scenario: list_models exposes first-party Claude Sonnet 5 models
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    When I start the UDS agent with no session
    And I send command "list_models" with id "models-1"
    And I close the UDS connection
    Then the agent output should contain a response command "list_models" with model "anthropic-api/claude-sonnet-5"
    And the agent output should contain a response command "list_models" with model "anthropic-oauth/claude-sonnet-5"

  Scenario: list_models reads models.json at runtime
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And a models registry with Fireworks model "accounts/fireworks/models/glm-5p2"
    When I start the UDS agent with no session
    And I send command "list_models" with id "models-1"
    And I close the UDS connection
    Then the agent output should contain a response command "list_models" with model "fireworks/accounts/fireworks/models/glm-5p2"

  Scenario: refresh_models reloads a discovered provider catalog for UDS clients
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And provider "openrouter" has auth, custom settings, and an old model
    And the OpenAI-compatible catalog for "openrouter" returns model "refreshed"
    When I start the UDS agent with no session
    And I send command "refresh_models" with id "refresh-1"
    And I send command "list_models" with id "models-1"
    And I close the UDS connection
    Then the agent output should contain a response command "refresh_models" with success true
    And the agent output should contain a response command "list_models" with model "openrouter/refreshed"

  Scenario: models.json provider can be selected and used without config restart
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And a models registry with Fireworks model "accounts/fireworks/models/glm-5p2"
    When I start the UDS agent with no session
    And I send set_model "fireworks/accounts/fireworks/models/glm-5p2"
    And I send prompt "use fireworks registry"
    And I close the UDS connection
    Then the agent output should contain a response command "set_model" with success true
    And the Fireworks provider should have received a chat completion request
