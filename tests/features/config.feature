@done
Feature: Configuration
  As a user
  I want to configure Quecto via a JSON file
  So that I can set API keys, models, and preferences

  Scenario: Load config from default path
    Given a config file at "~/.quecto/config.json" with content:
      """
      {
        "agents": {
          "defaults": {
            "model": "gpt-4",
            "max_tokens": 4096
          }
        },
        "providers": {
          "openai": {
            "api_key": "sk-test-123"
          }
        }
      }
      """
    When I load the config
    Then the model should be "gpt-4"
    And the max_tokens should be 4096
    And the OpenAI API key should be "sk-test-123"

  Scenario: Missing config fields use defaults
    Given a config file at "~/.quecto/config.json" with content:
      """
      {}
      """
    When I load the config
    Then the model should be "gpt-4"
    And the max_tokens should be 8192
    And the temperature should be 0.7
    And the workspace should be "~/.quecto/workspace"

  Scenario: Environment variables override config
    Given an environment variable "QUECTO_AGENTS_DEFAULTS_MODEL" set to "claude-opus-4-5"
    And a config file with model "gpt-4"
    When I load the config
    Then the model should be "claude-opus-4-5"

  Scenario: Workspace path expands tilde
    Given a config with workspace "~/.quecto/workspace"
    When I resolve the workspace path
    Then the workspace path should start with "/"
    And the workspace path should end with ".quecto/workspace"
