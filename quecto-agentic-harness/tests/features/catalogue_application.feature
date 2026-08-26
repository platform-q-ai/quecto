@wip @catalogue-application
Feature: Application catalogue resolve/query use cases and snapshot store (epic #1193, slice 2)
  As a maintainer of the provider/model catalogue
  I want application use cases that resolve sources into one published snapshot
  So that every consumer reads the same immutable catalogue generation

  Scenario: Sources resolve in precedence order into one published generation
    Given a catalogue source "builtin" at layer "built-in" defining model "openai-api/gpt-5" named "Builtin GPT"
    And a catalogue source "user" at layer "user-override" defining model "openai-api/gpt-5" named "My GPT"
    When the resolve-effective-catalogue use case runs
    Then the published snapshot has 1 model
    And the published model "openai-api/gpt-5" is named "My GPT"
    And the published snapshot generation is 1

  Scenario: A malformed source is isolated with a structured error
    Given a catalogue source "builtin" at layer "built-in" defining model "openai-api/gpt-5" named "Builtin GPT"
    And a malformed catalogue source "broken" at layer "user-defined" failing with "bad json"
    When the resolve-effective-catalogue use case runs
    Then the published snapshot has 1 model
    And the resolution reports a source error for "broken" containing "bad json"

  Scenario: When every source fails the last valid snapshot is retained
    Given a catalogue source "builtin" at layer "built-in" defining model "openai-api/gpt-5" named "Builtin GPT"
    When the resolve-effective-catalogue use case runs
    And the source "builtin" becomes malformed failing with "disk error"
    And the resolve-effective-catalogue use case runs
    Then the published snapshot has 1 model
    And the published snapshot generation is 1

  Scenario: Availability is derived from credential status without exposing secrets
    Given a catalogue source "builtin" at layer "built-in" defining model "openai-api/gpt-5" named "Builtin GPT"
    And no credential is available for provider "openai-api"
    When the resolve-effective-catalogue use case runs
    Then the published model "openai-api/gpt-5" is not runnable because a credential is missing
    And the published snapshot never contains the credential value "sk-secret-123"

  Scenario: Queries read the current snapshot only, never the sources
    Given a catalogue source "builtin" at layer "built-in" defining model "openai-api/gpt-5" named "Builtin GPT"
    When the resolve-effective-catalogue use case runs
    And the source "builtin" additionally defines model "openai-api/gpt-6" named "Next GPT"
    Then the query use case still lists 1 model for filter "all"

  Scenario: Every consumer surface projects the same snapshot generation
    Given a catalogue source "builtin" at layer "built-in" defining model "openai-api/gpt-5" named "Builtin GPT"
    When the resolve-effective-catalogue use case runs
    Then the shared model listing projection lists model "openai-api/gpt-5" at generation 1
