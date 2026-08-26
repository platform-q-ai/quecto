@catalogue-user-config
Feature: User-owned catalogue extension surface and hot reload (epic #1193, slice 5)
  As a Quecto user
  I want to add providers and models and override stale metadata in my own configuration
  So that the effective catalogue reflects my setup without recompiling or restarting

  # AC1a — data-only model add on an existing provider
  @done
  Scenario: Adding a model to an existing provider through the user catalogue file
    Given a user catalogue file adding model "gpt-5.5-preview" named "GPT 5.5 Preview" to provider "openai-api"
    When the effective catalogue is resolved from the user's configuration
    Then the published snapshot lists model "openai-api/gpt-5.5-preview" named "GPT 5.5 Preview"

  # AC1b — stable-ID metadata override of a built-in entry
  @done
  Scenario: Overriding a built-in model's metadata by stable ID
    Given a user catalogue file overriding model "openai-api/gpt-5.5" with name "My 5.5" and context window 999000
    When the effective catalogue is resolved from the user's configuration
    Then the published snapshot lists model "openai-api/gpt-5.5" named "My 5.5"
    And the published model "openai-api/gpt-5.5" has context window 999000

  # AC1b — an override replaces the built-in entry, never duplicates it
  @done
  Scenario: An override never duplicates the built-in entry
    Given a user catalogue file overriding model "openai-api/gpt-5.5" with name "My 5.5" and context window 999000
    When the effective catalogue is resolved from the user's configuration
    Then the published snapshot lists model "openai-api/gpt-5.5" exactly once

  # AC2 — data-only provider add on an existing transport reaches runnable
  @done
  Scenario: Adding a provider on an existing transport becomes runnable with a credential reference
    Given a user catalogue file adding provider "my-gateway" on transport "openai-completions" with model "custom-model"
    And that provider has base url "https://gw.example/v1"
    And that provider references credential "$MY_GATEWAY_KEY"
    And the environment provides "MY_GATEWAY_KEY"
    When the effective catalogue is resolved from the user's configuration
    Then the published model "my-gateway/custom-model" is runnable

  # AC3 — unsupported transport: known but unrunnable, structured reason
  @done
  Scenario: An entry on an unsupported transport is listed but not runnable
    Given a user catalogue file adding provider "wsprov" on transport "websocket-frames" with model "m2"
    When the effective catalogue is resolved from the user's configuration
    Then the published snapshot lists model "wsprov/m2"
    And the published model "wsprov/m2" is not runnable because its transport is unsupported

  # AC4a — hot reload: a valid edit is visible without restart
  @done
  Scenario: A valid edit to the user catalogue file is visible without restart
    Given a user catalogue file adding model "first" named "First" to provider "openai-api"
    And the effective catalogue has been resolved from the user's configuration
    And the user catalogue file has been rewritten to add model "second" named "Second" to provider "openai-api"
    When the effective catalogue is resolved from the user's configuration
    Then the published snapshot lists model "openai-api/second" named "Second"
    And the republished snapshot has a higher generation than the snapshot resolved before the rewrite

  # AC4a — hot reload is visible through the UDS models listing surface
  @done
  Scenario: A valid edit reaches the UDS models listing without a restart
    Given a user catalogue file adding model "first" named "First" to provider "openai-api"
    And the effective catalogue has been resolved from the user's configuration
    And the user catalogue file has been rewritten to add model "second" named "Second" to provider "openai-api"
    When the UDS models listing is requested
    Then the UDS models listing includes model "openai-api/second"

  # AC4b — hot reload: a malformed edit keeps the last valid generation and surfaces the error
  @done
  Scenario: A malformed edit keeps the last valid catalogue and surfaces the error
    Given a user catalogue file adding model "first" named "First" to provider "openai-api"
    And the effective catalogue has been resolved from the user's configuration
    And the user catalogue file has been rewritten to malformed JSON
    When the effective catalogue is resolved from the user's configuration
    Then the published snapshot still lists model "openai-api/first"
    And the resolution reports a user catalogue error mentioning "parse"

  # AC5 — no literal secrets in the override surface
  @done
  Scenario: A literal secret in the override surface is rejected with a structured error
    Given a user catalogue file overriding model "openai-api/gpt-5.5" with a literal secret "sk-live-secret123"
    When the effective catalogue is resolved from the user's configuration
    Then the resolution reports a user catalogue error mentioning "credential reference"
    And the published model "openai-api/gpt-5.5" keeps its built-in name

  # AC5/AC6 boundary — the legacy provider-level apiKey stays accepted for
  # compatibility (documented); only the new override surface is reference-only.
  @done
  Scenario: A legacy provider-level literal apiKey keeps working for compatibility
    Given a legacy user models file declaring provider "fireworks" with model "qwen3p7-plus" and a literal apiKey
    When the effective catalogue is resolved from the user's configuration
    Then the published model "fireworks/qwen3p7-plus" is runnable

  # AC6 — legacy models.json compatibility
  @done
  Scenario: A legacy models.json file keeps working unchanged
    Given a legacy user models file declaring provider "fireworks" with model "qwen3p7-plus"
    When the effective catalogue is resolved from the user's configuration
    Then the published snapshot lists model "fireworks/qwen3p7-plus"
