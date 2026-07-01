@done @runtime-models-phase3
Feature: Runtime model registry for discoverable models
  As a long-running UDS agent
  I want built-in and configured models to be loaded at runtime
  So that first-party and community models are discoverable and usable without restart

  # Acceptance criteria for #956:
  # - Claude Sonnet 5 is discoverable as both anthropic-api/claude-sonnet-5 and anthropic-oauth/claude-sonnet-5.
  # - Each Sonnet 5 registry entry exposes the published 1,000,000-token context window and 128,000-token output cap, not synthesized defaults.
  # - Selecting anthropic-oauth/claude-sonnet-5 succeeds through set_model, which is the same qualified model path used by spawned agents.
  # - Registry guard coverage is updated for model list, metadata, auth modes, ordering, and pricing.

  Scenario: list_models exposes first-party Claude Sonnet 5 models with published limits
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    When I start the UDS agent with no session
    And I send command "list_models" with id "models-1"
    And I close the UDS connection
    Then the agent output should contain a response command "list_models" with model "anthropic-api/claude-sonnet-5"
    And the agent output should contain a response command "list_models" with model "anthropic-oauth/claude-sonnet-5"
    And the response command "list_models" model "anthropic-api/claude-sonnet-5" should have context window 1000000
    And the response command "list_models" model "anthropic-api/claude-sonnet-5" should have max tokens 128000
    And the response command "list_models" model "anthropic-oauth/claude-sonnet-5" should have context window 1000000
    And the response command "list_models" model "anthropic-oauth/claude-sonnet-5" should have max tokens 128000

  Scenario: get_models exposes the same Claude Sonnet 5 published limits
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    When I start the UDS agent with no session
    And I send command "get_models" with id "models-2"
    And I close the UDS connection
    Then the agent output should contain a response command "get_models" with model "anthropic-api/claude-sonnet-5"
    And the agent output should contain a response command "get_models" with model "anthropic-oauth/claude-sonnet-5"
    And the response command "get_models" model "anthropic-api/claude-sonnet-5" should have context window 1000000
    And the response command "get_models" model "anthropic-api/claude-sonnet-5" should have max tokens 128000
    And the response command "get_models" model "anthropic-oauth/claude-sonnet-5" should have context window 1000000
    And the response command "get_models" model "anthropic-oauth/claude-sonnet-5" should have max tokens 128000

  Scenario: Claude Sonnet 5 OAuth can be selected by qualified name
    Given a temp base directory
    And the credential store has a valid OAuth credential for anthropic account "acct-sonnet-5"
    And a config file with an OpenAI provider pointing at a mock server
    When I start the UDS agent with no session
    And I send set_model "anthropic-oauth/claude-sonnet-5"
    And I close the UDS connection
    Then the agent output should contain a response command "set_model" with success true

  Scenario: Claude Sonnet 5 OAuth is accepted by the spawned agent entrypoint
    Given a mocked Anthropic workspace is configured
    And the credential store has a valid OAuth credential for anthropic account "acct-sonnet-5"
    And the mock expects Authorization header "Bearer sk-ant-oat01-sonnet-5" and returns "SONNET_5_OAUTH_OK"
    When I run quecto agent --model "anthropic-oauth/claude-sonnet-5" -s - -m "Reply with SONNET_5_OAUTH_OK"
    Then the exit code should be 0
    And stdout should contain "SONNET_5_OAUTH_OK"

  Scenario: list_models reads models.json at runtime
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And a models registry with Fireworks model "accounts/fireworks/models/glm-5p2"
    When I start the UDS agent with no session
    And I send command "list_models" with id "models-1"
    And I close the UDS connection
    Then the agent output should contain a response command "list_models" with model "fireworks/accounts/fireworks/models/glm-5p2"

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
