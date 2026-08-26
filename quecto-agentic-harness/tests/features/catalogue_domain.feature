@done @catalogue-domain
Feature: Canonical catalogue domain model (epic #1193, slice 1)
  As a maintainer of the provider/model catalogue
  I want canonical domain types with pure merge, validation, and override rules
  So that every consumer resolves providers and models from one authoritative model

  Scenario: Typed model references round-trip the existing string identifiers
    Given the qualified model identifier "openai-api/gpt-5"
    When I parse it into a typed model reference
    Then the reference serializes back to exactly "openai-api/gpt-5"
    And the reference names provider "openai-api" and model "gpt-5"

  Scenario: Blank identifiers are rejected at construction
    When I try to construct a provider id from "   "
    Then the catalogue id construction is rejected

  Scenario: API-key and OAuth identities are distinct provider identities
    Given a provider descriptor "anthropic-api" authenticating with an API key
    And a provider descriptor "anthropic-oauth" authenticating with OAuth via "anthropic-oauth"
    Then the two provider identities are distinct

  Scenario: Later source layers override earlier ones by stable identity
    Given a built-in catalogue layer defining model "openai-api/gpt-5" named "Builtin GPT"
    And a user-override catalogue layer defining model "openai-api/gpt-5" named "My GPT"
    When I resolve the catalogue layers into a snapshot
    Then the resolved snapshot has 1 model
    And the resolved model "openai-api/gpt-5" is named "My GPT"

  Scenario: Layer precedence follows the documented order, not insertion order
    Given a user-defined catalogue layer defining model "openai-api/gpt-5" named "User GPT"
    And a built-in catalogue layer defining model "openai-api/gpt-5" named "Builtin GPT"
    When I resolve the catalogue layers into a snapshot
    Then the resolved model "openai-api/gpt-5" is named "User GPT"

  Scenario: Invalid entries are rejected without corrupting the rest of a resolution
    Given a built-in catalogue layer defining model "openai-api/gpt-5" named "Builtin GPT"
    And a built-in catalogue layer entry whose model reference names a different provider
    When I resolve the catalogue layers into a snapshot
    Then the resolved snapshot has 1 model
    And 1 catalogue entry was rejected

  Scenario: Non-runnable availability carries a structured reason
    Given a catalogue entry for "custom/local" that is configured but missing a credential
    When I resolve the catalogue layers into a snapshot
    Then the resolved model "custom/local" is not runnable because a credential is missing
