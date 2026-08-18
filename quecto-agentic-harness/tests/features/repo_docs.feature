@docs @done
Feature: Repository documentation
  As a maintainer
  I want committed documentation to match the current workspace state
  So that users see current version, packaging, and license information without obsolete planning artifacts

  @docs
  Scenario: README documents current release metadata and private license
    When I read the repository file "README.md"
    Then the output should describe the current package version
    And the output should contain "## License"
    And the output should contain "LicenseRef-Proprietary"
    And the output should contain "private repository"

  @docs
  Scenario: README runtime details match current code
    When I read the repository file "README.md"
    Then the output should contain "`max_context_tokens = 200000`"
    And the output should contain "`context_collapse_after_tool_calls = 50`"
    And the output should not contain "`max_context_tokens = 1000000`"
    And the output should not contain "QUECTO_* environment variables (including API keys) are stripped"

  @docs
  Scenario: README UDS protocol lists current commands and events
    When I read the repository file "README.md"
    Then the output should contain "get_subagents"
    And the output should contain "subagent_notification"
    And the output should contain "subagent_state_changed"
    And the output should contain "workflow_state"
    And the output should not contain "AgentCommand` enum (15 variants"

  @docs
  Scenario: Obsolete planning artifacts are removed from product docs
    When I inspect obsolete repository planning artifact paths
    Then the obsolete planning documents should be absent

  @docs
  Scenario: Harness architecture map covers Phase 0 hardening surfaces
    When I read the repository file "docs/architecture/harness-architecture-map.md"
    Then the harness architecture map should cover the Phase 0 hardening surfaces
    And the harness architecture map should record baseline hardening checks

  @docs
  Scenario: Protocol capability matrix lists baseline UDS capabilities
    When I read the repository file "docs/architecture/protocol-capability-matrix.md"
    Then the protocol capability matrix should include the baseline UDS capabilities

  @docs
  Scenario: Phase 0 hardening docs are discoverable from protocol and ADR docs
    When I inspect the Phase 0 hardening documentation links
    Then the Phase 0 hardening documentation links should resolve

  @docs
  Scenario: Workflow docs keep pure-move refactors reviewable
    When I read the repository file "README.md"
    Then the workflow docs should describe pure-move refactors as separate PRs before or after motivating behavior
