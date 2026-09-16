@done @issue-1966
Feature: Configuration discovery
  As a user
  I want Quecto to prefer a config.json in my current directory
  So that a project can carry its own configuration without changing my global defaults

  Scenario: A config.json in the current directory is preferred over the global configuration
    Given a config file at "~/.quecto/config.json" with content:
      """
      {"agents":{"defaults":{"model":"global-model"}}}
      """
    And a config file named "config.json" in the current directory with content:
      """
      {"agents":{"defaults":{"model":"local-model"}}}
      """
    When I run quecto with arguments "status"
    Then the exit code should be 0
    And the reported config path should be the current directory's "config.json"
    And the output should contain "Model:     local-model"

  Scenario: Without a local config.json the global configuration is used
    Given a config file at "~/.quecto/config.json" with content:
      """
      {"agents":{"defaults":{"model":"global-model"}}}
      """
    When I run quecto with arguments "status"
    Then the exit code should be 0
    And the reported config path should be the global "config.json"
    And the output should contain "Model:     global-model"

  Scenario: An explicit --config selection wins over the local config.json
    Given a config file named "config.json" in the current directory with content:
      """
      {"agents":{"defaults":{"model":"local-model"}}}
      """
    And a config file named "explicit.json" in the current directory with content:
      """
      {"agents":{"defaults":{"model":"explicit-model"}}}
      """
    When I run quecto status with --config pointing at the current directory's "explicit.json"
    Then the exit code should be 0
    And the output should contain "Model:     explicit-model"

  Scenario: A parent directory's config.json is not discovered
    Given a config file at "~/.quecto/config.json" with content:
      """
      {"agents":{"defaults":{"model":"global-model"}}}
      """
    And a config file named "config.json" in the parent of the current directory with content:
      """
      {"agents":{"defaults":{"model":"parent-model"}}}
      """
    When I run quecto with arguments "status"
    Then the exit code should be 0
    And the output should contain "Model:     global-model"

  Scenario: An invalid local config.json is an error rather than a silent fallback
    Given a config file at "~/.quecto/config.json" with content:
      """
      {"agents":{"defaults":{"model":"global-model"}}}
      """
    And a config file named "config.json" in the current directory with content:
      """
      {"agents":{"defaults":{"model":
      """
    When I run quecto with arguments "status"
    Then the exit code should be 1
    And the stderr should contain "failed to load config"
    And the stderr should name the current directory's "config.json"
    And the output should not contain "global-model"

  Scenario: An invalid local config.json stops an agent run with an error naming the file
    Given a config file at "~/.quecto/config.json" with content:
      """
      {"providers":{"openai":{"api_key":"sk-global"}}}
      """
    And a config file named "config.json" in the current directory with content:
      """
      {"providers":{"openai":{"api_key":
      """
    When I run quecto with arguments "agent -m hello"
    Then the exit code should be 1
    And the stderr should contain "failed to load config"
    And the stderr should name the current directory's "config.json"

  Scenario: An agent run uses the local config.json ahead of the global configuration
    Given a config file at "~/.quecto/config.json" with content:
      """
      {"providers":{"openai":{"api_key":"sk-global"}}}
      """
    And a config file named "config.json" in the current directory with content:
      """
      {"providers":{"openai":{"api_key":""},"anthropic":{"api_key":""}}}
      """
    When I run quecto with arguments "agent -m hello"
    Then the exit code should be 1
    And the stderr should contain "no LLM providers"

  Scenario: A local config.json that is not a regular file is an error
    Given a config file at "~/.quecto/config.json" with content:
      """
      {"agents":{"defaults":{"model":"global-model"}}}
      """
    And a directory named "config.json" in the current directory
    When I run quecto with arguments "status"
    Then the exit code should be 1
    And the stderr should contain "config.json"
    And the stderr should contain "not a regular file"
    And the output should not contain "global-model"

  Scenario: An unreadable local config.json is an error
    Given a config file at "~/.quecto/config.json" with content:
      """
      {"agents":{"defaults":{"model":"global-model"}}}
      """
    And a dangling symlink named "config.json" in the current directory
    When I run quecto with arguments "status"
    Then the exit code should be 1
    And the stderr should contain "config.json"
    And the stderr should contain "cannot be read"
    And the output should not contain "global-model"
