@done @issue-1847
Feature: Change the active model
  As a UDS client
  I want set_model to switch the session's model with its limits and effort in one step
  So that a model switch never leaves the loop clamped or configured for the previous model

  # The use case over fake ports: limits from the generation it publishes,
  # the runtime's verdict from the same generation, and the effort reset.

  Scenario: A switch reads the model's declared limits from the generation it just published
    Given a catalogue input defining model "acme/limited" with max tokens 50 and context window 1234
    When the active model is changed to "acme/limited"
    Then the loop runs on "acme/limited" with max tokens 50 and context window 1234
    And the published catalogue generation is 1

  Scenario: A model added to the inputs since the last read is switchable without a refresh
    Given a catalogue input defining model "acme/first" with no declared limits
    When the active model is changed to "acme/first"
    And the catalogue input additionally defines model "acme/second" with max tokens 10 and context window 20
    And the active model is changed to "acme/second"
    Then the loop runs on "acme/second" with max tokens 10 and context window 20
    And the published catalogue generation is 2

  Scenario: The reply carries the runtime's verdict and the switch proceeds regardless
    Given a catalogue input defining model "acme/keyless" with no declared limits
    And no credential is available for "acme/keyless"
    And a runtime generation composed over those inputs
    When the active model is changed to "acme/keyless"
    Then the switch verdict is not runnable because a credential is missing
    And the loop runs on "acme/keyless" with no declared limits

  Scenario: An unknown reference switches with no limits and an unknown verdict
    Given a runtime generation composed over those inputs
    When the active model is changed to "openrouter/anything"
    Then the switch verdict is unknown model "openrouter/anything"
    And the loop runs on "openrouter/anything" with no declared limits

  Scenario: A switch resets the effort for the new model
    Given a catalogue input defining model "acme/limited" with max tokens 50 and context window 1234
    And the loop's effort is "xhigh"
    When the active model is changed to "acme/limited"
    Then the loop's effort is "low"

  # End to end over UDS with the real catalogue.

  Scenario: set_model over UDS switches the loop and reports the catalogue's verdict
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And a models registry with Fireworks model "accounts/fireworks/models/glm-5p2"
    When I start the UDS agent with no session
    And I send set_model "fireworks/accounts/fireworks/models/glm-5p2"
    And I send command "get_state" with id "gs-1"
    And I close the UDS connection
    Then the agent output should contain a response command "set_model" with success true
    And the set_model response selection status should be "ok" on provider "fireworks"
    And the get_state response model should be "fireworks/accounts/fireworks/models/glm-5p2"
