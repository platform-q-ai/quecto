@done @runtime-reload-phase2b
Feature: Provider reload wiring for models/providers (Phase 2b)
  As a long-running UDS agent
  I want provider config changes to be reloaded without restart
  So that a model/provider added by an agent can be selected and used on the next turn

  Scenario: set_model reloads providers before selecting a newly configured provider
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the config file will be updated to add a Fireworks provider before the UDS command loop
    When I start the UDS agent with no session
    And I send set_model "fireworks/accounts/fireworks/models/glm-5p2"
    And I send prompt "use fireworks"
    And I close the UDS connection
    Then the agent output should contain a response command "set_model" with success true
    And the Fireworks provider should have received a chat completion request

  Scenario: prompt reloads providers at top of turn before using the configured model
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the config default model is "fireworks/accounts/fireworks/models/glm-5p2"
    And the config file will be updated to add a Fireworks provider before the UDS command loop
    When I start the UDS agent with no session
    And I send prompt "use fireworks"
    And I close the UDS connection
    Then the Fireworks provider should have received a chat completion request

  Scenario: explicit reload reports malformed config without dropping the last-good provider
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the config file is replaced with invalid JSON before the UDS command loop
    When I start the UDS agent with no session
    And I send command "reload" with id "reload-1"
    And I close the UDS connection
    Then the agent output should contain a response command "reload" with success false
