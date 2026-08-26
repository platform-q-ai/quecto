@done @catalogue-runtime
Feature: Application-owned provider runtime composition and model selection (epic #1193, slice 3)
  As a maintainer of the provider/model catalogue
  I want the provider runtime composed by an application use case
  So that catalogue and routing are always one coherent generation

  Scenario: Composition publishes runtime and catalogue as one generation
    Given a catalogue source defining model "openai-api/gpt-5"
    And a provider factory that composes a runtime named "router"
    When the provider runtime is composed and published
    Then the published runtime generation matches the published catalogue generation
    And the published runtime provider is named "router"

  Scenario: Failed composition reports the failure
    Given a published runtime for model "openai-api/gpt-5"
    And the provider factory now fails
    When the provider runtime is composed and published
    Then the composition fails carrying the factory's error

  Scenario: Failed composition retains the previous valid runtime and catalogue
    Given a published runtime for model "openai-api/gpt-5"
    And the provider factory now fails
    When the provider runtime is composed and published
    Then the previously published runtime and catalogue are retained

  Scenario: A listed runnable model resolves to the catalogue's provider identity
    Given a published runtime for model "openai-api/gpt-5"
    When model "openai-api/gpt-5" is selected
    Then the selection succeeds with provider "openai-api" transport "openai-completions" auth "api-key"
    And the selection generation matches the published catalogue generation

  Scenario: Selecting an unknown model returns a structured unknown-model reason
    Given a published runtime for model "openai-api/gpt-5"
    When model "openai-api/no-such-model" is selected
    Then the selection fails because model "openai-api/no-such-model" is unknown

  Scenario: Selecting a model without a credential returns a missing-credential reason
    Given a catalogue source defining model "openai-api/gpt-5"
    And no runtime credential is available for provider "openai-api"
    And a provider factory that composes a runtime named "router"
    And the provider runtime has been composed and published
    When model "openai-api/gpt-5" is selected
    Then the selection fails because a credential is missing

  Scenario: API-key and OAuth identities are never silently swapped by selection
    Given a catalogue source defining model "openai-api/gpt-5" with auth "api-key"
    And a catalogue source defining model "openai/gpt-5" with auth "oauth"
    And a provider factory that composes a runtime named "router"
    And the provider runtime has been composed and published
    When model "openai/gpt-5" is selected
    Then the selection succeeds with provider "openai" transport "openai-completions" auth "oauth"
    When model "openai-api/gpt-5" is selected
    Then the selection succeeds with provider "openai-api" transport "openai-completions" auth "api-key"
