@done @issue-2024
Feature: Default model and effort per repository
  As a user working across several repositories
  I want a repository's .quecto/config.json to pin the model and effort its agents start on
  So that a new agent in that repository starts on them, other repositories are unaffected, and I can pin them from the running session

  Background:
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the global config file sets "agents.defaults.model" to "openai-api/gpt-5.6-sol"

  # ── The overlay at startup ─────────────────────────────────────────────────

  Scenario: An agent started in a repository with a trusted overlay starts on the overlay's model and effort
    Given a repo-local overlay in the current directory with content:
      """
      {"agents":{"defaults":{"model":"openai-api/gpt-5.6-luna","effort":"high"}}}
      """
    And the repo-local overlay is trusted
    When a production UDS agent is started in the current directory
    Then the production agent's state should report model "openai-api/gpt-5.6-luna"
    And the production agent's state should report effort "high"

  Scenario: A sibling repository without an overlay starts on the global default
    Given a repo-local overlay in the current directory with content:
      """
      {"agents":{"defaults":{"model":"openai-api/gpt-5.6-luna"}}}
      """
    And the repo-local overlay is trusted
    And a sibling repository directory "sibling"
    When a production UDS agent is started in the sibling directory
    Then the production agent's state should report model "openai-api/gpt-5.6-sol"

  # ── set_model / set_effort persist ─────────────────────────────────────────

  Scenario: set_model with persist local switches the session and writes the repository overlay
    When a production UDS agent is started in the current directory
    And the production agent is sent set_model "openai-api/gpt-5.6-luna" with persist "local"
    Then the production agent's reply should succeed
    And the production agent's reply should report persisted scope "local" at the current directory's ".quecto/config.json"
    And the production agent's state should report model "openai-api/gpt-5.6-luna"
    And the current directory's ".quecto/config.json" should set "agents.defaults.model" to "openai-api/gpt-5.6-luna"
    And the global config file should be byte-identical to its previous content
    When I run quecto with arguments "status"
    Then the reported overlay path should be the current directory's ".quecto/config.json" marked "trusted"
    And the output should contain "Model:     openai-api/gpt-5.6-luna"
    When a production UDS agent is started in the current directory
    Then the production agent's state should report model "openai-api/gpt-5.6-luna"

  Scenario: set_model with persist global writes the qualified id into the global file
    When a production UDS agent is started in the current directory
    And the production agent is sent set_model provider "openai-api" modelId "gpt-5.6-luna" with persist "global"
    Then the production agent's reply should succeed
    And the production agent's reply should report persisted scope "global" at the global "config.json"
    And the global config file should set "agents.defaults.model" to "openai-api/gpt-5.6-luna"
    And the global config file should set "providers.openai.api_key" to "sk-test-key"
    And the current directory's ".quecto/config.json" should not exist
    When a production UDS agent is started in the sibling directory
    Then the production agent's state should report model "openai-api/gpt-5.6-luna"

  Scenario: Under an explicit --config, persist global writes that file and persist local is refused
    Given an explicit config file "explicit.json" copied from the global config file
    When a production UDS agent is started in the current directory with that explicit config
    And the production agent is sent set_model "openai-api/gpt-5.6-luna" with persist "global"
    Then the production agent's reply should succeed
    And the production agent's reply should report persisted scope "global" at the explicit config file
    And the explicit config file should set "agents.defaults.model" to "openai-api/gpt-5.6-luna"
    And the global config file should be byte-identical to its previous content
    And the production agent's state should report model "openai-api/gpt-5.6-luna"
    When the production agent is sent set_effort "high" with persist "local"
    Then the production agent's reply should fail
    And the production agent's reply error should contain "no repo-local overlay"
    And the current directory's ".quecto/config.json" should not exist
    And the production agent's state should report effort "low"

  Scenario: set_effort with persist local writes the repository overlay
    When a production UDS agent is started in the current directory
    And the production agent is sent set_effort "high" with persist "local"
    Then the production agent's reply should succeed
    And the production agent's reply should report persisted scope "local" at the current directory's ".quecto/config.json"
    And the production agent's state should report effort "high"
    And the current directory's ".quecto/config.json" should set "agents.defaults.effort" to "high"
    And the global config file should be byte-identical to its previous content
    When a production UDS agent is started in the current directory
    Then the production agent's state should report effort "high"

  Scenario: Without persist set_model changes only the session
    When a production UDS agent is started in the current directory
    And the production agent is sent set_model "openai-api/gpt-5.6-luna" with persist "none"
    Then the production agent's reply should succeed
    And the production agent's reply should report nothing persisted
    And the production agent's state should report model "openai-api/gpt-5.6-luna"
    And the current directory's ".quecto/config.json" should not exist
    And the global config file should be byte-identical to its previous content

  # ── Refusals (the S1 writer's semantics) ──────────────────────────────────

  Scenario: Persisting into an untrusted overlay is refused with the remedy and the session is left unchanged
    Given a repo-local overlay in the current directory with content:
      """
      {"agents":{"defaults":{"model":"someone-elses-model"}}}
      """
    When a production UDS agent is started in the current directory
    And the production agent is sent set_model "openai-api/gpt-5.6-luna" with persist "local"
    Then the production agent's reply should fail
    And the production agent's reply error should contain "not trusted"
    And the production agent's reply error should contain "quecto config trust"
    And the production agent's state should report model "openai-api/gpt-5.6-sol"
    And the current directory's ".quecto/config.json" should be byte-identical to its previous content

  Scenario: Persisting through a symbolic link at the overlay location is refused
    Given a trusted overlay in another directory with content:
      """
      {"agents":{"defaults":{"model":"borrowed-model"}}}
      """
    And the current directory's ".quecto/config.json" is a symbolic link to that overlay
    When a production UDS agent is started in the current directory
    And the production agent is sent set_model "openai-api/gpt-5.6-luna" with persist "local"
    Then the production agent's reply should fail
    And the production agent's reply error should contain "symbolic link"
    And the other directory's overlay should be byte-identical to its previous content
    And the production agent's state should report model "openai-api/gpt-5.6-sol"

  Scenario: An unknown persist scope is refused before anything changes
    When a production UDS agent is started in the current directory
    And the production agent is sent set_model "openai-api/gpt-5.6-luna" with persist "everywhere"
    Then the production agent's reply should fail
    And the production agent's reply error should contain "persist must be"
    And the production agent's reply error should contain "everywhere"
    And the production agent's state should report model "openai-api/gpt-5.6-sol"

  Scenario: A bare model id that no provider resolves cannot be persisted
    When a production UDS agent is started in the current directory
    And the production agent is sent set_model "nobody-knows-this" with persist "local"
    Then the production agent's reply should fail
    And the production agent's reply error should contain "provider/model"
    And the production agent's state should report model "openai-api/gpt-5.6-sol"
    And the current directory's ".quecto/config.json" should not exist

  Scenario: A provider the published catalogue lists no models for cannot be persisted, while an unlisted id on a listed provider can
    When a production UDS agent is started in the current directory
    And the production agent is sent set_model "nobody/some-model" with persist "local"
    Then the production agent's reply should fail
    And the production agent's reply error should contain "the published catalogue lists no models for `nobody`"
    And the production agent's state should report model "openai-api/gpt-5.6-sol"
    And the current directory's ".quecto/config.json" should not exist
    When the production agent is sent set_model "openai-api/not-listed-yet" with persist "local"
    Then the production agent's reply should succeed
    And the production agent's state should report model "openai-api/not-listed-yet"
    And the current directory's ".quecto/config.json" should set "agents.defaults.model" to "openai-api/not-listed-yet"

  # ── The CLI path and its rollback ─────────────────────────────────────────

  Scenario: quecto config unset removes a pinned default and the global default applies again
    When I run quecto with the arguments:
      """
      config set agents.defaults.model '"openai-api/gpt-5.6-luna"'
      """
    Then the exit code should be 0
    When I run quecto with arguments "config get --effective agents.defaults.model"
    Then the printed JSON should be "openai-api/gpt-5.6-luna"
    When I run quecto with arguments "config unset agents.defaults.model"
    Then the exit code should be 0
    And the stdout should name the current directory's ".quecto/config.json"
    When I run quecto with arguments "config get --effective agents.defaults.model"
    Then the printed JSON should be "openai-api/gpt-5.6-sol"
    When I run quecto with arguments "config get --local"
    Then the printed JSON should have "agents.defaults" set to {}
    When I run quecto with arguments "status"
    Then the reported overlay path should be the current directory's ".quecto/config.json" marked "trusted"

  Scenario: quecto config unset of a key that is not set is an error naming the layer
    When I run quecto with arguments "config unset --global agents.defaults.effort"
    Then the exit code should be 1
    And the stderr should contain "agents.defaults.effort"
    And the stderr should contain "not set"
    And the global config file should be byte-identical to its previous content
