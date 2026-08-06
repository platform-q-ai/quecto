@container-config-fallback
Feature: Container spawns fall back to the parent's config
  As a parent agent
  I want container spawns without an explicit config argument to use my own effective config path
  So that agents need not hunt for the config location while explicit config still wins

  @done @container-spawn
  Scenario: Container true without a config argument uses the parent's config path
    Given script-managed subagent spawning is available through the parent's config path with default script "default"
    When I spawn script-managed subagent "container-fallback-cfg" without a config argument and task "CONTAINER_FALLBACK_MARKER"
    Then the spawn result should not be an error
    And the spawn result should include an environment reference
    And the script-managed runtime should have used container script "default"
    And child "container-fallback-cfg" should receive "CONTAINER_FALLBACK_MARKER"

  @done @container-spawn
  Scenario: Explicit config still overrides the parent's config path
    Given script-managed subagent spawning is available through an unusable parent config path with default script "default"
    When I spawn script-managed subagent "container-explicit-cfg" with default selection and task "CONTAINER_EXPLICIT_CFG_MARKER"
    Then the spawn result should not be an error
    And the spawn result should include an environment reference
    And the script-managed runtime should have used container script "default"
    And child "container-explicit-cfg" should receive "CONTAINER_EXPLICIT_CFG_MARKER"

  @done @container-spawn
  Scenario: Container spawn without any config source keeps the clear error
    Given live subagent spawning is available
    When I spawn script-managed subagent "container-no-cfg" without a config argument and task "CONTAINER_NO_CFG_MARKER"
    Then the spawn result should fail because container spawning requires a config
    And the spawn result should not include an environment reference
