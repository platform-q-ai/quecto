@wip @catalogue-user-config
Feature: User-owned catalogue extension surface and hot reload (epic #1193, slice 5)
  As a Quecto user
  I want to add providers and models and override stale metadata in my own configuration
  So that the effective catalogue reflects my setup without recompiling or restarting

  # AC1a — data-only model add on an existing provider
  Scenario: Adding a model to an existing provider through the user catalogue file
    Given a user catalogue file adding model "gpt-5.5-preview" named "GPT 5.5 Preview" to provider "openai-api"
    When the effective catalogue is resolved from the user base directory
    Then the published snapshot lists model "openai-api/gpt-5.5-preview" named "GPT 5.5 Preview"

  # AC1b — stable-ID metadata override of a built-in entry
  Scenario: Overriding a built-in model's metadata by stable ID
    Given a user catalogue file overriding model "openai-api/gpt-5.5" with name "My 5.5" and context window 999000
    When the effective catalogue is resolved from the user base directory
    Then the published model "openai-api/gpt-5.5" is named "My 5.5"
    And the published model "openai-api/gpt-5.5" has context window 999000
    And the published snapshot lists model "openai-api/gpt-5.5" exactly once

  # AC2 — data-only provider add on an existing transport reaches runnable
  Scenario: Adding a provider on an existing transport becomes runnable with a credential reference
    Given a user catalogue file adding provider "my-gateway" on transport "openai-completions" with base url "https://gw.example/v1" and credential reference "$MY_GATEWAY_KEY"
    And the environment provides "MY_GATEWAY_KEY"
    When the effective catalogue is resolved from the user base directory
    Then the published model "my-gateway/custom-model" is runnable

  # AC3 — unsupported transport: known but unrunnable, structured reason
  Scenario: An entry on an unsupported transport is listed but not runnable
    Given a user catalogue file adding provider "wsprov" on transport "websocket-frames" with model "m2"
    When the effective catalogue is resolved from the user base directory
    Then the published snapshot lists model "wsprov/m2"
    And the published model "wsprov/m2" is not runnable because its transport is unsupported

  # AC4a — hot reload: a valid edit is visible without restart
  Scenario: A valid edit to the user catalogue file is visible without restart
    Given a user catalogue file adding model "first" named "First" to provider "openai-api"
    And the effective catalogue has been resolved from the user base directory
    When the user catalogue file is rewritten to add model "second" named "Second" to provider "openai-api"
    And the effective catalogue is resolved from the user base directory
    Then the published snapshot lists model "openai-api/second" named "Second"
    And the published snapshot generation has advanced

  # AC4b — hot reload: a malformed edit keeps the last valid generation and surfaces the error
  Scenario: A malformed edit keeps the last valid catalogue and surfaces the error
    Given a user catalogue file adding model "first" named "First" to provider "openai-api"
    And the effective catalogue has been resolved from the user base directory
    When the user catalogue file is rewritten to malformed JSON
    And the effective catalogue is resolved from the user base directory
    Then the published snapshot still lists model "openai-api/first"
    And the resolution reports a user catalogue error mentioning "parse"

  # AC5 — no literal secrets in catalogue files
  Scenario: A literal secret in the override surface is rejected with a structured error
    Given a user catalogue file overriding model "openai-api/gpt-5.5" with a literal secret "sk-live-secret123"
    When the effective catalogue is resolved from the user base directory
    Then the resolution reports a user catalogue error mentioning "credential reference"
    And the published model "openai-api/gpt-5.5" keeps its built-in name

  # AC6 — legacy models.json compatibility
  Scenario: A legacy models.json file keeps working unchanged
    Given a legacy user models file declaring provider "fireworks" with model "qwen3p7-plus"
    When the effective catalogue is resolved from the user base directory
    Then the published snapshot lists model "fireworks/qwen3p7-plus"
