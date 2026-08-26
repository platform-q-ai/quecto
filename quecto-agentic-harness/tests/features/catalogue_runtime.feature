@wip @catalogue-runtime
Feature: Application-owned provider runtime composition and model selection (epic #1193, slice 3)
  As a maintainer of the provider/model catalogue
  I want the provider runtime composed by an application use case
  So that catalogue and routing are always one coherent generation

  Scenario: Composition publishes runtime and catalogue as one generation
    Given a runtime catalogue source "builtin" at layer "built-in" defining model "openai-api/gpt-5" named "Builtin GPT"
    And a provider factory that composes a runtime named "router"
    When the provider runtime is composed and published
    Then the published runtime generation matches the published catalogue generation
    And the published runtime provider is named "router"

  Scenario: Failed composition retains the previous valid runtime and catalogue
    Given a runtime catalogue source "builtin" at layer "built-in" defining model "openai-api/gpt-5" named "Builtin GPT"
    And a provider factory that composes a runtime named "router"
    And the provider runtime has been composed and published
    And the provider factory now fails with "boom"
    When the provider runtime is composed and published
    Then the composition reports a runtime error containing "boom"
    And the published runtime generation is 1
    And the published runtime provider is named "router"

  Scenario: A listed runnable model resolves to the catalogue's provider identity
    Given a runtime catalogue source "builtin" at layer "built-in" defining model "openai-api/gpt-5" named "Builtin GPT"
    And a provider factory that composes a runtime named "router"
    And the provider runtime has been composed and published
    When model "openai-api/gpt-5" is selected
    Then the selection succeeds with provider "openai-api" transport "openai-completions" auth "api-key"
    And the selection generation matches the published catalogue generation

  Scenario: Selecting an unknown model returns a structured unknown-model reason
    Given a runtime catalogue source "builtin" at layer "built-in" defining model "openai-api/gpt-5" named "Builtin GPT"
    And a provider factory that composes a runtime named "router"
    And the provider runtime has been composed and published
    When model "openai-api/no-such-model" is selected
    Then the selection fails because the model is unknown

  Scenario: Selecting a model without a credential returns a missing-credential reason
    Given a runtime catalogue source "builtin" at layer "built-in" defining model "openai-api/gpt-5" named "Builtin GPT"
    And no runtime credential is available for provider "openai-api"
    And a provider factory that composes a runtime named "router"
    And the provider runtime has been composed and published
    When model "openai-api/gpt-5" is selected
    Then the selection fails because a credential is missing

  Scenario: API-key and OAuth identities are never silently swapped by selection
    Given a runtime catalogue source "builtin" at layer "built-in" defining model "openai-api/gpt-5" named "API GPT" with auth "api-key"
    And a runtime catalogue source "oauth" at layer "built-in" defining model "openai/gpt-5" named "OAuth GPT" with auth "oauth"
    And a provider factory that composes a runtime named "router"
    And the provider runtime has been composed and published
    When model "openai/gpt-5" is selected
    Then the selection succeeds with provider "openai" transport "openai-completions" auth "oauth"
