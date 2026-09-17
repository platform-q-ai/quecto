@done @issue-1846
Feature: Refresh model catalogue sources over UDS
  As a UDS client
  I want refresh_models to pull each provider's live model list into the catalogue
  So that new models appear in the listing without editing models.json or restarting

  Scenario: refresh_models reports a per-source outcome and publishes the discovered models
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And provider "openrouter" has auth, custom settings, and an old model
    And the OpenAI-compatible catalog for "openrouter" returns models "alpha" and "beta"
    When I start the UDS agent with no session
    And I send command "refresh_models" with id "rf-1"
    And I send command "list_models" with id "models-1"
    And I close the UDS connection
    Then the agent output should contain a response command "refresh_models" with success true
    And the refresh_models response reports source "openrouter" as "updated" with 2 models
    And the refresh_models response carries a published generation
    And the list_models generation is exactly one after the refresh_models generation
    And the agent output should contain a response command "list_models" with model "openrouter/alpha"

  Scenario: refresh_models with a source nobody configured is a failed outcome, not an error
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    When I start the UDS agent with no session
    And I send refresh_models for source "no-such-provider" with id "rf-1"
    And I close the UDS connection
    Then the agent output should contain a response command "refresh_models" with success true
    And the refresh_models response reports source "no-such-provider" as "failed"
    And the refresh_models response carries no published generation
