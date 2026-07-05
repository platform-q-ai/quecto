@provider-auth-modes @done
Feature: Explicit auth modes for models.json providers
  As a user with both OAuth (monthly plan) and API-key (token billing) access
  I want to declare each provider's auth mode explicitly in models.json
  So that I can run OAuth and API-key providers for the same vendor side by side
  without the kernel ever silently switching billing modes.

  Scenario: models.json provider declares apiKey auth and is constructed
    Given a temp base directory
    And a models registry with an anthropic-api provider using api key "$ANTHROPIC_API_KEY"
    When I build the agent provider
    Then the router should expose a provider named "anthropic-api"

  Scenario: models.json provider declares oauth auth referencing a kernel oauth provider
    Given a temp base directory
    And a stored anthropic OAuth credential
    And a models registry with an anthropic-oauth provider referencing oauth provider "anthropic"
    When I build the agent provider
    Then the router should expose a provider named "anthropic-oauth"

  Scenario: oauth reference to an unknown kernel provider is rejected
    Given a temp base directory
    And a models registry with a provider referencing oauth provider "cohere"
    When I build the agent provider
    Then provider construction should fail with "not a kernel OAuth provider"

  Scenario: OAuth and API-key providers for the same vendor coexist
    Given a temp base directory
    And a stored anthropic OAuth credential
    And a models registry with both anthropic-oauth and anthropic-api providers
    When I build the agent provider
    Then the router should expose a provider named "anthropic-oauth"
    And the router should expose a provider named "anthropic-api"

  Scenario: list_models reports the auth mode of each model
    Given a temp base directory
    And a stored anthropic OAuth credential
    And a models registry with both anthropic-oauth and anthropic-api providers
    When I start the UDS agent with no session
    And I send command "list_models" with id "auth-1"
    And I close the UDS connection
    Then the list_models response should mark "anthropic-oauth" models as auth "oauth"
    And the list_models response should mark "anthropic-api" models as auth "apiKey"
