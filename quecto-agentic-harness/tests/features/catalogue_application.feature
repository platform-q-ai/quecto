@done @catalogue-application
Feature: Effective catalogue resolution and snapshot queries (epic #1193, slice 2)
  As a maintainer of the provider/model catalogue
  I want catalogue sources resolved into one published snapshot
  So that every consumer reads the same immutable catalogue generation

  Scenario: Sources resolve in precedence order into one published generation
    Given a catalogue source "builtin" at layer "built-in" defining model "openai-api/gpt-5" named "Builtin GPT"
    And a catalogue source "user" at layer "user-override" defining model "openai-api/gpt-5" named "My GPT"
    When the effective catalogue is resolved and published
    Then the published snapshot has 1 model
    And the published model "openai-api/gpt-5" is named "My GPT"
    And the published snapshot generation is 1

  Scenario: A second successful resolution publishes the next generation
    Given a catalogue source "builtin" at layer "built-in" defining model "openai-api/gpt-5" named "Builtin GPT"
    And the effective catalogue has been resolved and published
    When the effective catalogue is resolved and published
    Then the published snapshot generation is 2
    And the published snapshot has 1 model

  Scenario: A malformed source is isolated with a structured error
    Given a catalogue source "builtin" at layer "built-in" defining model "openai-api/gpt-5" named "Builtin GPT"
    And a malformed catalogue source "broken" at layer "user-defined" failing with "bad json"
    When the effective catalogue is resolved and published
    Then the published snapshot has 1 model
    And the resolution reports a source error for "broken" containing "bad json"

  Scenario: A failed re-resolution retains the last valid snapshot
    Given a catalogue source "builtin" at layer "built-in" defining model "openai-api/gpt-5" named "Builtin GPT"
    And the effective catalogue has been resolved and published
    And the source "builtin" becomes malformed failing with "disk error"
    When the effective catalogue is resolved and published
    Then the published snapshot has 1 model
    And the published snapshot generation is 1

  Scenario: Availability is derived from credential status
    Given a catalogue source "builtin" at layer "built-in" defining model "openai-api/gpt-5" named "Builtin GPT"
    And no credential is available for provider "openai-api"
    When the effective catalogue is resolved and published
    Then the published model "openai-api/gpt-5" is not runnable because a credential is missing

  Scenario: The published snapshot never contains credential material
    Given a catalogue source "builtin" at layer "built-in" defining model "openai-api/gpt-5" named "Builtin GPT"
    And the credential store holds the secret "sk-secret-123" for provider "openai-api"
    When the effective catalogue is resolved and published
    Then the published snapshot does not contain the secret "sk-secret-123"

  Scenario: Listing models loads the inputs afresh so edits are visible
    Given a catalogue source "builtin" at layer "built-in" defining model "openai-api/gpt-5" named "Builtin GPT"
    When the models are listed through the use case
    Then the listing shows 1 model at generation 1
    Given the source "builtin" additionally defines model "openai-api/gpt-6" named "Next GPT"
    When the models are listed through the use case
    Then the listing shows 2 models at generation 2
    And the listing contains model "openai-api/gpt-6" named "Next GPT"

  Scenario: Listing models marks a model that only lacks a credential as not runnable
    Given a catalogue source "builtin" at layer "built-in" defining model "openai-api/gpt-5" named "Builtin GPT"
    And a catalogue source "extras" at layer "built-in" defining model "anthropic/opus" named "Opus"
    And no credential is available for provider "openai-api"
    When the models are listed through the use case
    Then the listing shows 2 models at generation 1
    And the listed model "anthropic/opus" is runnable
    And the listed model "openai-api/gpt-5" is not runnable

  Scenario: Listing models reports a source that failed to load instead of listing over it
    Given a catalogue source "builtin" at layer "built-in" defining model "openai-api/gpt-5" named "Builtin GPT"
    When the models are listed through the use case
    Then the listing shows 1 model at generation 1
    Given the source "builtin" becomes malformed failing with "expected value at line 1"
    When the models are listed through the use case
    Then the listing reports source "builtin" unavailable with "expected value at line 1"
    And the published snapshot has 1 model

