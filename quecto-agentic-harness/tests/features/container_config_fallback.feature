@container-config-fallback
Feature: Container spawns fall back to the parent's config
  As a parent agent
  I want container spawns without an explicit config argument to use my own effective config path
  So that agents need not hunt for the config location while explicit config still wins

  @done @container-spawn
  Scenario: A container spawn without an explicit config uses the parent's config path
    Given script-managed subagent spawning is available through the parent's config path with default script "default"
    When I spawn script-managed subagent "container-fallback-cfg" without a config argument and task "CONTAINER_FALLBACK_MARKER"
    Then the spawn result should not be an error
    And the spawn result should include an environment reference
    And the script-managed runtime should have used container script "default"
    And child "container-fallback-cfg" should receive "CONTAINER_FALLBACK_MARKER"

  @done @container-spawn @serial
  Scenario: A container spawn without an explicit config falls back to the inherited runtime config
    Given script-managed subagent spawning is available through the inherited runtime config with default script "default"
    When I spawn script-managed subagent "container-inherited-cfg" without a config argument and task "CONTAINER_INHERITED_MARKER"
    Then the spawn result should not be an error
    And the spawn result should include an environment reference
    And the script-managed runtime should have used container script "default"
    And child "container-inherited-cfg" should receive "CONTAINER_INHERITED_MARKER"

  @done @container-spawn
  Scenario: An explicit config argument still overrides the parent's config path
    Given script-managed subagent spawning is available through an unusable parent config path with default script "default"
    When I spawn script-managed subagent "container-explicit-cfg" with an explicit config argument and task "CONTAINER_EXPLICIT_CFG_MARKER"
    Then the spawn result should not be an error
    And the spawn result should include an environment reference
    And the script-managed runtime should have used container script "default"
    And child "container-explicit-cfg" should receive "CONTAINER_EXPLICIT_CFG_MARKER"

  @done @container-spawn @serial
  Scenario: A container spawn without any config source keeps the clear error
    Given script-managed subagent spawning is available with no parent config path
    When I spawn script-managed subagent "container-no-cfg" without a config argument and task "CONTAINER_NO_CFG_MARKER"
    Then the spawn result should fail because container spawning requires a config
    And the spawn result should not include an environment reference
