@docs @wip
Feature: Repository documentation
  As a maintainer
  I want committed documentation to match the current workspace state
  So that users see current version, packaging, and license information without obsolete planning artifacts

  @docs
  Scenario: README documents current release metadata and private license
    When I read the repository file "README.md"
    Then the output should contain "Current version: **0.81.18**"
    And the output should contain "## License"
    And the output should contain "LicenseRef-Proprietary"
    And the output should contain "private repository"

  @docs
  Scenario: README runtime details match current code
    When I read the repository file "README.md"
    Then the output should contain "`max_context_tokens = 200000`"
    And the output should contain "`context_collapse_after_turns = 50`"
    And the output should not contain "1000000"
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
