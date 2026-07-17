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

  Scenario: Reuse a workflow step from a relative file
    Given a workflow step file "steps/shared.json" with content:
      """
      {"key":"shared","label":"Shared step","phase":"green","guidance":"Reuse this"}
      """
    And a config file at "~/.quecto/config.json" with content:
      """
      {"workflow":{"templates":[{"id":"test","label":"Test","description":"Test workflow","steps":["steps/shared"]}]}}
      """
    When I load the config
    Then workflow step 1 should be "shared" in phase "green" with guidance "Reuse this"

  Scenario: Adapt a reused workflow step for one template
    Given a workflow step file "shared.json" with content:
      """
      {"key":"shared","label":"Shared step","phase":"green","guidance":"Base guidance"}
      """
    And a config file at "~/.quecto/config.json" with content:
      """
      {"workflow":{"templates":[{"id":"test","label":"Test","description":"Test workflow","steps":[{"ref":"shared.json","key":"adapted","phase":"review","guidance":"Review it"}]}]}}
      """
    When I load the config
    Then workflow step 1 should be "adapted" in phase "review" with guidance "Review it"

  Scenario: Override every structural field of a reused workflow step
    Given a workflow step file "shared.json" with content:
      """
      {"key":"shared","label":"Shared step","phase":"green","guidance":"From the file"}
      """
    And a config file at "~/.quecto/config.json" with content:
      """
      {"workflow":{"templates":[{"id":"test","label":"Test","description":"Test workflow","steps":[{"ref":"shared.json","key":"adapted","label":"Adapted","phase":"review"}]}]}}
      """
    When I load the config
    Then workflow step 1 should be "adapted" in phase "review" with guidance "From the file"

  Scenario: Report the referenced workflow step file when it cannot be loaded
    Given a config file at "~/.quecto/config.json" with content:
      """
      {"workflow":{"templates":[{"id":"test","label":"Test","description":"Test workflow","steps":["steps/missing"]}]}}
      """
    When I try to load the config
    Then config loading should fail with "steps/missing.json"

  Scenario: Reject workflow step files that reference other files
    Given a workflow step file "recursive.json" with content:
      """
      {"ref":"other"}
      """
    And a config file at "~/.quecto/config.json" with content:
      """
      {"workflow":{"templates":[{"id":"test","label":"Test","description":"Test workflow","steps":["recursive"]}]}}
      """
    When I try to load the config
    Then config loading should fail with "recursive references are not allowed"

  # --- workflow-composable-templates PRD §3.2 slice 2: template directory
  # --- discovery (workflow.dir → ./.quecto/workflows → ~/.quecto/workflows
  # --- → inline workflow.templates fallback)

  @workflow-dir
  Scenario: Discover templates from a configured workflow directory
    Given a workflow template file "wf/speedy.json" with content:
      """
      {"label":"Speedy","description":"A dropped-in template","steps":[{"key":"one","label":"One","phase":"green","guidance":"go"}]}
      """
    And a config file at "~/.quecto/config.json" with content:
      """
      {"workflow":{"dir":"wf"}}
      """
    When I discover workflow templates
    Then the only discovered workflow template should be "speedy"

  @workflow-dir
  Scenario: A template dropped into the repo-local workflow directory appears without any config edit
    Given a workflow template file ".quecto/workflows/foo.json" with content:
      """
      {"label":"Foo","description":"d","steps":[{"key":"one","label":"One","phase":"green","guidance":"go"}]}
      """
    And an empty config file
    When I discover workflow templates
    Then the only discovered workflow template should be "foo"

  @workflow-dir
  Scenario: The home workflow directory is used when the repository has none
    Given a workflow template file "~/.quecto/workflows/bar.json" with content:
      """
      {"label":"Bar","description":"d","steps":[{"key":"one","label":"One","phase":"green","guidance":"go"}]}
      """
    And an empty config file
    When I discover workflow templates
    Then the only discovered workflow template should be "bar"

  @workflow-dir
  Scenario: Inline templates remain the fallback, without a warning, when no workflow directory exists
    Given a config file at "~/.quecto/config.json" with content:
      """
      {"workflow":{"templates":[{"id":"inline_tpl","label":"Inline","description":"d","steps":[{"key":"one","label":"One","phase":"green","guidance":"go"}]}]}}
      """
    When I discover workflow templates
    Then the only discovered workflow template should be "inline_tpl"
    And no workflow discovery warning should be issued

  @workflow-dir
  Scenario: A workflow directory wins over inline templates with a startup warning
    Given a workflow template file ".quecto/workflows/from_dir.json" with content:
      """
      {"label":"Dir","description":"d","steps":[{"key":"one","label":"One","phase":"green","guidance":"go"}]}
      """
    And a config file at "~/.quecto/config.json" with content:
      """
      {"workflow":{"templates":[{"id":"inline_tpl","label":"Inline","description":"d","steps":[{"key":"one","label":"One","phase":"green","guidance":"go"}]}]}}
      """
    When I discover workflow templates
    Then the only discovered workflow template should be "from_dir"
    And a workflow discovery warning should mention "inline"

  @workflow-dir
  Scenario: Files under the steps subfolder are never templates
    Given a workflow template file ".quecto/workflows/real.json" with content:
      """
      {"label":"Real","description":"d","steps":[{"key":"one","label":"One","phase":"green","guidance":"go"}]}
      """
    And a workflow template file ".quecto/workflows/steps/reviews/shared.json" with content:
      """
      {"key":"shared","label":"Shared","phase":"review","guidance":"defined once"}
      """
    And an empty config file
    When I discover workflow templates
    Then the only discovered workflow template should be "real"

  @workflow-dir
  Scenario: A discovered template resolves step references relative to the workflow directory
    Given a workflow template file ".quecto/workflows/real.json" with content:
      """
      {"label":"Real","description":"d","steps":["steps/reviews/shared"]}
      """
    And a workflow template file ".quecto/workflows/steps/reviews/shared.json" with content:
      """
      {"key":"shared","label":"Shared","phase":"review","guidance":"defined once"}
      """
    And an empty config file
    When I discover workflow templates
    Then discovered template "real" step 1 should have guidance "defined once"

  @workflow-dir
  Scenario: An unloadable template file fails discovery naming the file
    Given a workflow template file ".quecto/workflows/broken.json" with content:
      """
      not json {
      """
    And an empty config file
    When I try to discover workflow templates
    Then workflow discovery should fail with "broken.json"

  Scenario: Missing config fields use defaults
    Given a config file at "~/.quecto/config.json" with content:
      """
      {}
      """
    When I load the config
    Then the model should be "gpt-5.5"
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

  @pr3-performance
  Scenario: Max session messages default and override are loaded
    Given a config file at "~/.quecto/config.json" with content:
      """
      {}
      """
    When I load the config
    Then the max_session_messages should be 200

    Given a config file at "~/.quecto/config.json" with content:
      """
      {
        "agents": {
          "defaults": {
            "max_session_messages": 12
          }
        }
      }
      """
    When I load the config
    Then the max_session_messages should be 12

  # --- #416: Effort level in config and env var ---

  Scenario: Effort level is loaded from config file
    Given a config file at "~/.quecto/config.json" with content:
      """
      {
        "agents": {
          "defaults": {
            "effort": "medium"
          }
        }
      }
      """
    When I load the config
    Then the effort should be "medium"

  Scenario: Effort level defaults to None when omitted
    Given a config file at "~/.quecto/config.json" with content:
      """
      {}
      """
    When I load the config
    Then the effort should be unset

  Scenario: Environment variable overrides effort in config
    Given an environment variable "QUECTO_AGENTS_DEFAULTS_EFFORT" set to "high"
    And a config file at "~/.quecto/config.json" with content:
      """
      {
        "agents": {
          "defaults": {
            "effort": "low"
          }
        }
      }
      """
    When I load the config
    Then the effort should be "high"

  Scenario: Invalid effort value in config produces error
    Given a config file at "~/.quecto/config.json" with content:
      """
      {
        "agents": {
          "defaults": {
            "effort": "turbo"
          }
        }
      }
      """
    When I load the config and validate effort
    Then the effort validation should fail with "invalid effort level"
    And the effort validation should fail with "expected one of: none, low, medium, high, xhigh, max"

  # --- #629: OpenAI-compatible custom endpoints alongside OpenAI OAuth ---

  @issue-629
  Scenario: OpenAI-compatible provider endpoints are loaded from config
    Given a config file at "~/.quecto/config.json" with content:
      """
      {
        "providers": {
          "openai_compatible": {
            "endpoints": [
              {
                "prefix": "spark",
                "api_base": "http://127.0.0.1:8000/v1",
                "api_key": "sk-spark",
                "allow_remote_http": true
              }
            ]
          }
        }
      }
      """
    When I load the config
    Then the OpenAI-compatible provider should have endpoint "spark" with api_base "http://127.0.0.1:8000/v1"

  @issue-629
  Scenario: OpenAI provider disable_codex_routing flag is loaded from config
    Given a config file at "~/.quecto/config.json" with content:
      """
      {
        "providers": {
          "openai": {
            "api_key": "sk-custom",
            "api_base": "http://127.0.0.1:8000/v1",
            "auth_method": "api_key",
            "disable_codex_routing": true
          }
        }
      }
      """
    When I load the config
    Then the OpenAI provider should have disable_codex_routing enabled
