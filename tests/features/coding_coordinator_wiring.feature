@wip
Feature: Coordinator wiring via config flag
  As the composition root
  I want to choose between inline and subagent coordinator modes via config
  So that the coding_job tool is backed by the correct implementation

  The `tools.coding.coordinator_mode` config field controls which
  implementation backs the `coding_job` tool: `inline` (in-process
  CodingJobTool via CodingLifecycleDriver) or `subagent` (delegation
  tool via file-based IPC to a coordinator subprocess).

  # --- Config defaults ---

  Scenario: Default config uses inline coordinator mode
    Given a default config
    Then the coordinator mode should be "inline"

  Scenario: Config can be set to subagent mode
    Given a config with coordinator mode "subagent"
    Then the coordinator mode should be "subagent"

  # --- Tool registration ---

  Scenario: Inline mode registers coding_job tool and returns a driver
    Given a tool registry with workspace
    When I build the coding tool in inline mode
    Then the registry should contain a "coding_job" tool
    And the lifecycle driver should be present

  Scenario: Subagent mode registers coding_job tool without a driver
    Given a tool registry with workspace
    When I build the coding tool in subagent mode
    Then the registry should contain a "coding_job" tool
    And the lifecycle driver should not be present

  # --- Backward compatibility ---

  Scenario: Both modes register a tool with the same name
    Given a tool registry with workspace
    When I build the coding tool in inline mode
    Then the registry should contain a "coding_job" tool
    Given a fresh tool registry with workspace
    When I build the coding tool in subagent mode
    Then the registry should contain a "coding_job" tool
