@done
Feature: Onboarding
  As a new user
  I want to initialize Quecto with a single command
  So that the config and workspace are ready to use

  Scenario: First-time onboarding creates config and workspace
    Given no config file exists at "~/.quecto/config.json"
    When I run quecto with arguments "onboard"
    Then the exit code should be 0
    And a config file should exist at "~/.quecto/config.json"
    And a workspace directory should exist at "~/.quecto/workspace"
    And the output should contain "quecto is ready"

  Scenario: Workspace contains required template files
    Given no config file exists at "~/.quecto/config.json"
    When I run quecto with arguments "onboard"
    Then the workspace should contain "AGENTS.md"
    And the workspace should contain "IDENTITY.md"
    And the workspace should contain "SOUL.md"
    And the workspace should contain "TOOLS.md"
    And the workspace should contain "USER.md"

  Scenario: Onboarding with existing config prompts for overwrite
    Given a config file already exists at "~/.quecto/config.json"
    When I run quecto with arguments "onboard"
    Then the output should contain "Config already exists"

  Scenario: Default config has sensible defaults
    Given no config file exists at "~/.quecto/config.json"
    When I run quecto with arguments "onboard"
    Then the config should have model "gpt-5.2-codex"
    And the config should have max_tokens 8192
    And the config should have temperature 0.7
    And the config should have restrict_to_workspace true
