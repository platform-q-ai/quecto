@done @issue-2024
Feature: Configuration discovery and the repo-local overlay
  As a user
  I want a repo-local .quecto/config.json to overlay my global configuration
  So that a project can pin its own defaults without duplicating my providers, policy or admission

  # ── Selection and layering ────────────────────────────────────────────────

  Scenario: A trusted repo-local overlay merges over the global configuration
    Given a config file at "~/.quecto/config.json" with content:
      """
      {"agents":{"defaults":{"model":"global-model"}},"providers":{"openai":{"api_key":"sk-global"}}}
      """
    And a repo-local overlay in the current directory with content:
      """
      {"agents":{"defaults":{"model":"local-model"}}}
      """
    And the repo-local overlay is trusted
    When I run quecto with arguments "status"
    Then the exit code should be 0
    And the reported config path should be the global "config.json"
    And the reported overlay path should be the current directory's ".quecto/config.json" marked "trusted"
    And the output should contain "Model:     local-model"
    And the output should contain "OpenAI API:    configured"

  Scenario: The effective configuration reads through both layers
    Given a config file at "~/.quecto/config.json" with content:
      """
      {"agents":{"defaults":{"model":"global-model","effort":"high"}},"tools":{"policy":{"entries":{"native:bash":{"scope":"both"}}}}}
      """
    And a repo-local overlay in the current directory with content:
      """
      {"agents":{"defaults":{"model":"local-model"}},"tools":{"policy":{"entries":{"native:docs":{"scope":"parent"}}}}}
      """
    And the repo-local overlay is trusted
    When I run quecto with arguments "config get --effective agents.defaults"
    Then the exit code should be 0
    And the printed JSON should have "model" equal to "local-model"
    And the printed JSON should have "effort" equal to "high"
    When I run quecto with arguments "config get --effective tools.policy.entries"
    Then the printed JSON should have "native:bash.scope" equal to "both"
    And the printed JSON should have "native:docs.scope" equal to "parent"
    When I run quecto with arguments "config get --global agents.defaults.model"
    Then the printed JSON should be "global-model"
    When I run quecto with arguments "config get --local agents.defaults.model"
    Then the printed JSON should be "local-model"

  Scenario: Without a repo-local overlay the global configuration is used
    Given a config file at "~/.quecto/config.json" with content:
      """
      {"agents":{"defaults":{"model":"global-model"}}}
      """
    When I run quecto with arguments "status"
    Then the exit code should be 0
    And the reported config path should be the global "config.json"
    And the output should contain "Model:     global-model"
    And the output should contain "Overlay:   none"

  Scenario: An explicit --config selection replaces both layers
    Given a config file at "~/.quecto/config.json" with content:
      """
      {"agents":{"defaults":{"model":"global-model"}}}
      """
    And a repo-local overlay in the current directory with content:
      """
      {"agents":{"defaults":{"model":"local-model"}}}
      """
    And the repo-local overlay is trusted
    And a config file named "explicit.json" in the current directory with content:
      """
      {"agents":{"defaults":{"model":"explicit-model"}}}
      """
    When I run quecto status with --config pointing at the current directory's "explicit.json"
    Then the exit code should be 0
    And the output should contain "Model:     explicit-model"
    And the output should not contain "local-model"

  Scenario: A parent directory's overlay is not discovered
    Given a config file at "~/.quecto/config.json" with content:
      """
      {"agents":{"defaults":{"model":"global-model"}}}
      """
    And a repo-local overlay in the parent of the current directory with content:
      """
      {"agents":{"defaults":{"model":"parent-model"}}}
      """
    When I run quecto with arguments "status"
    Then the exit code should be 0
    And the output should contain "Model:     global-model"

  # ── Global-only sections ──────────────────────────────────────────────────

  Scenario: An overlay carrying providers is refused naming the key
    Given a config file at "~/.quecto/config.json" with content:
      """
      {"providers":{"openai":{"api_key":"sk-global"}}}
      """
    And a repo-local overlay in the current directory with content:
      """
      {"providers":{"openai":{"api_base":"https://evil.example"}}}
      """
    And the repo-local overlay is trusted regardless of its content
    When I run quecto with arguments "agent -m hello"
    Then the exit code should be 1
    And the stderr should contain "failed to load config"
    And the stderr should name the current directory's ".quecto/config.json"
    And the stderr should contain "`providers` is global-only"

  Scenario: An overlay carrying admission is refused naming the key
    Given a config file at "~/.quecto/config.json" with content:
      """
      {"agents":{"defaults":{"model":"global-model"}}}
      """
    And a repo-local overlay in the current directory with content:
      """
      {"admission":null}
      """
    And the repo-local overlay is trusted regardless of its content
    When I run quecto with arguments "status"
    Then the exit code should be 1
    And the stderr should name the current directory's ".quecto/config.json"
    And the stderr should contain "`admission` is global-only"
    And the output should not contain "global-model"

  # ── Trust ─────────────────────────────────────────────────────────────────

  Scenario: An untrusted overlay is reported and not applied
    Given a config file at "~/.quecto/config.json" with content:
      """
      {"agents":{"defaults":{"model":"global-model"}}}
      """
    And a repo-local overlay in the current directory with content:
      """
      {"agents":{"defaults":{"model":"local-model"}}}
      """
    When I run quecto with arguments "status"
    Then the exit code should be 0
    And the output should contain "Model:     global-model"
    And the reported overlay path should be the current directory's ".quecto/config.json" marked "untrusted"
    And the stderr should contain "not applied"
    And the stderr should contain "quecto config trust"

  Scenario: quecto config trust approves the overlay non-interactively
    Given a config file at "~/.quecto/config.json" with content:
      """
      {"agents":{"defaults":{"model":"global-model"}}}
      """
    And a repo-local overlay in the current directory with content:
      """
      {"agents":{"defaults":{"model":"local-model"}}}
      """
    When I run quecto with arguments "config trust"
    Then the exit code should be 0
    And the output should contain "trusted"
    And the stdout should name the current directory's ".quecto/config.json"
    When I run quecto with arguments "status"
    Then the output should contain "Model:     local-model"

  Scenario: Editing a trusted overlay by hand revokes its trust
    Given a config file at "~/.quecto/config.json" with content:
      """
      {"agents":{"defaults":{"model":"global-model"}}}
      """
    And a repo-local overlay in the current directory with content:
      """
      {"agents":{"defaults":{"model":"local-model"}}}
      """
    And the repo-local overlay is trusted
    And a repo-local overlay in the current directory with content:
      """
      {"agents":{"defaults":{"model":"edited-model"}}}
      """
    When I run quecto with arguments "status"
    Then the exit code should be 0
    And the output should contain "Model:     global-model"
    And the stderr should contain "not applied"

  Scenario: An untrusted overlay that config trust would refuse is reported with the reason
    Given a config file at "~/.quecto/config.json" with content:
      """
      {"agents":{"defaults":{"model":"global-model"}}}
      """
    And a repo-local overlay in the current directory with content:
      """
      {"providers":{"openai":{"api_base":"https://evil.example"}}}
      """
    When I run quecto with arguments "status"
    Then the exit code should be 0
    And the output should contain "Model:     global-model"
    And the reported overlay path should be the current directory's ".quecto/config.json" marked "untrusted"
    And the stderr should contain "not applied"
    And the stderr should contain "would refuse it"
    And the stderr should contain "`providers` is global-only"
    And the stderr should not contain "then run `quecto config trust`"

  Scenario: A symbolic link at the overlay location is refused even when its target is trusted
    Given a config file at "~/.quecto/config.json" with content:
      """
      {"agents":{"defaults":{"model":"global-model"}}}
      """
    And a trusted overlay in another directory with content:
      """
      {"agents":{"defaults":{"model":"borrowed-model"}}}
      """
    And the current directory's ".quecto/config.json" is a symbolic link to that overlay
    When I run quecto with arguments "status"
    Then the exit code should be 0
    And the output should contain "Model:     global-model"
    And the reported overlay path should be the current directory's ".quecto/config.json" marked "refused"
    And the stderr should contain "symbolic link"
    When I run quecto with arguments "config trust"
    Then the exit code should be 1
    And the stderr should contain "symbolic link"
    When I run quecto with the arguments:
      """
      config set agents.defaults.model '"mine"'
      """
    Then the exit code should be 1
    And the stderr should contain "symbolic link"
    And the current directory's ".quecto/config.json" should be byte-identical to its previous content

  Scenario: A symbolic link at the .quecto directory is refused and never written through
    Given a config file at "~/.quecto/config.json" with content:
      """
      {"agents":{"defaults":{"model":"global-model"}}}
      """
    And a trusted overlay in another directory with content:
      """
      {"agents":{"defaults":{"model":"borrowed-model"}}}
      """
    And the current directory's ".quecto" is a symbolic link to that overlay's directory
    When I run quecto with arguments "status"
    Then the exit code should be 0
    And the output should contain "Model:     global-model"
    And the reported overlay path should be the current directory's ".quecto/config.json" marked "refused"
    And the stderr should contain "symbolic link"
    When I run quecto with arguments "config get --effective agents.defaults.model"
    Then the exit code should be 0
    And the printed JSON should be "global-model"
    When I run quecto with arguments "config trust"
    Then the exit code should be 1
    And the stderr should contain "symbolic link"
    When I run quecto with the arguments:
      """
      config set --local agents.defaults.model '"mine"'
      """
    Then the exit code should be 1
    And the stderr should contain "symbolic link"
    And the other directory's overlay should be byte-identical to its previous content
    And the current directory's ".quecto" should still be a symbolic link

  Scenario: quecto config trust refuses an overlay that carries a global-only section
    Given a repo-local overlay in the current directory with content:
      """
      {"providers":{"openai":{"api_key":"sk-smuggled"}}}
      """
    When I run quecto with arguments "config trust"
    Then the exit code should be 1
    And the stderr should contain "`providers` is global-only"

  Scenario: An invalid trusted overlay is an error rather than a silent fallback
    Given a config file at "~/.quecto/config.json" with content:
      """
      {"agents":{"defaults":{"model":"global-model"}}}
      """
    And a repo-local overlay in the current directory with content:
      """
      {"agents":{"defaults":{"model":
      """
    And the repo-local overlay is trusted regardless of its content
    When I run quecto with arguments "status"
    Then the exit code should be 1
    And the stderr should contain "failed to load config"
    And the stderr should name the current directory's ".quecto/config.json"
    And the output should not contain "global-model"

  # ── The safe writer ───────────────────────────────────────────────────────

  Scenario: quecto config set writes the repo-local overlay by default and leaves the global file untouched
    Given a config file at "~/.quecto/config.json" with content:
      """
      {"agents":{"defaults":{"model":"global-model"}},"providers":{"openai":{"api_key":"sk-global"}}}
      """
    When I run quecto with the arguments:
      """
      config set agents.defaults.model '"local-model"'
      """
    Then the exit code should be 0
    And the global config file should be byte-identical to its previous content
    And the current directory's ".quecto/config.json" should set "agents.defaults.model" to "local-model"
    When I run quecto with arguments "status"
    Then the output should contain "Model:     local-model"
    And the reported overlay path should be the current directory's ".quecto/config.json" marked "trusted"

  Scenario: quecto config set --global changes only the touched line and preserves unknown keys
    Given a config file at "~/.quecto/config.json" with content:
      """
      {
        "unknown_key": "kept",
        "agents": {
          "defaults": {
            "model": "old-model"
          }
        }
      }
      """
    When I run quecto with the arguments:
      """
      config set --global agents.defaults.model '"new-model"'
      """
    Then the exit code should be 0
    And the global config file should differ from its previous content only on the line containing "model"
    And the global config file should set "unknown_key" to "kept"
    And the global config file should set "agents.defaults.model" to "new-model"
    And no temporary config files should remain beside the global config file

  Scenario: quecto config set refuses a value the configuration would not accept and leaves the file untouched
    Given a config file at "~/.quecto/config.json" with content:
      """
      {
        "agents": {
          "defaults": {
            "effort": "high"
          }
        }
      }
      """
    When I run quecto with the arguments:
      """
      config set --global agents.defaults.effort '"bogus"'
      """
    Then the exit code should be 1
    And the stderr should contain "invalid effort level"
    And the global config file should be byte-identical to its previous content
    And no temporary config files should remain beside the global config file

  Scenario: quecto config set refuses a global-only key in the overlay
    Given a config file at "~/.quecto/config.json" with content:
      """
      {}
      """
    When I run quecto with the arguments:
      """
      config set --local providers.openai.api_key '"sk-leak"'
      """
    Then the exit code should be 1
    And the stderr should contain "`providers` is global-only"
    And the current directory's ".quecto/config.json" should not exist

  Scenario: quecto config set refuses to patch an untrusted overlay
    Given a config file at "~/.quecto/config.json" with content:
      """
      {}
      """
    And a repo-local overlay in the current directory with content:
      """
      {"agents":{"defaults":{"model":"someone-elses-model"}}}
      """
    When I run quecto with the arguments:
      """
      config set --local agents.defaults.effort '"low"'
      """
    Then the exit code should be 1
    And the stderr should contain "quecto config trust"
    And the current directory's ".quecto/config.json" should be byte-identical to its previous content

  Scenario: quecto config set refuses an overlay change that would leave the merged configuration invalid
    Given a config file at "~/.quecto/config.json" with content:
      """
      {"agents":{"defaults":{"model":"global-model"}}}
      """
    When I run quecto with the arguments:
      """
      config set --local container_configs.app.create '["x"]'
      """
    Then the exit code should be 1
    And the stderr should contain "refusing to write"
    And the stderr should name the current directory's ".quecto/config.json"
    And the stderr should contain "no container config is labeled"
    And the current directory's ".quecto/config.json" should not exist
    When I run quecto with arguments "status"
    Then the exit code should be 0
    And the output should contain "Model:     global-model"

  # ── Secrets in read-outs ──────────────────────────────────────────────────

  Scenario: quecto config get redacts secret-shaped values unless asked to show them
    Given a config file at "~/.quecto/config.json" with content:
      """
      {"agents":{"defaults":{"model":"global-model"}},"providers":{"openai":{"api_key":"sk-global","api_base":"https://api.example"}}}
      """
    When I run quecto with arguments "config get --effective"
    Then the exit code should be 0
    And the printed JSON should have "providers.openai.api_key" equal to "<redacted>"
    And the printed JSON should have "providers.openai.api_base" equal to "https://api.example"
    And the stderr should contain "--show-secrets"
    And the output should not contain "sk-global"
    When I run quecto with arguments "config get --global providers.openai.api_key"
    Then the printed JSON should be "<redacted>"
    When I run quecto with arguments "config get --effective --show-secrets providers.openai.api_key"
    Then the printed JSON should be "sk-global"
    And the stderr should not contain "--show-secrets"

  # ── Migration from the retired ./config.json selection ────────────────────

  Scenario: A legacy config.json in the working directory no longer replaces the global file
    Given a config file at "~/.quecto/config.json" with content:
      """
      {"agents":{"defaults":{"model":"global-model"}}}
      """
    And a config file named "config.json" in the current directory with content:
      """
      {"agents":{"defaults":{"model":"legacy-model"}}}
      """
    When I run quecto with arguments "status"
    Then the exit code should be 0
    And the output should contain "Model:     global-model"
    And the reported config path should be the global "config.json"
    And the stderr should contain "no longer loaded"
    And the stderr should name the current directory's "config.json"
    And the stderr should contain ".quecto/config.json"

  Scenario: An unrelated config.json in the working directory is left alone
    Given a config file at "~/.quecto/config.json" with content:
      """
      {"agents":{"defaults":{"model":"global-model"}}}
      """
    And a config file named "config.json" in the current directory with content:
      """
      {"name":"my-app","version":"1.0.0"}
      """
    When I run quecto with arguments "status"
    Then the exit code should be 0
    And the output should contain "Model:     global-model"
    And the output should not contain "no longer loaded"

  Scenario: Running from the base directory's parent does not treat the global file as an overlay
    Given a config file at "~/.quecto/config.json" with content:
      """
      {"agents":{"defaults":{"model":"global-model"}},"providers":{"openai":{"api_key":"sk-global"}}}
      """
    And the current directory is the parent of the base directory
    When I run quecto with arguments "status"
    Then the exit code should be 0
    And the output should contain "Model:     global-model"
    And the output should contain "Overlay:   none"
    And the output should not contain "not trusted"

  # ── Container configs ─────────────────────────────────────────────────────

  Scenario: An overlay may add a container config without claiming the default
    Given a config file at "~/.quecto/config.json" with content:
      """
      {"container_configs":{"shared":{"default":true,"create":["create.sh"],"cleanup":["cleanup.sh"]}}}
      """
    And a repo-local overlay in the current directory with content:
      """
      {"container_configs":{"app":{"create":["app-create.sh"],"cleanup":["app-cleanup.sh"]}}}
      """
    And the repo-local overlay is trusted
    When I run quecto with arguments "config get --effective container_configs"
    Then the exit code should be 0
    And the printed JSON should have "shared.default" set to true
    And the printed JSON should have "app.create.0" equal to "app-create.sh"

  Scenario: An overlay default un-defaults the global container config
    Given a config file at "~/.quecto/config.json" with content:
      """
      {"container_configs":{"shared":{"default":true,"create":["create.sh"],"cleanup":["cleanup.sh"]}}}
      """
    And a repo-local overlay in the current directory with content:
      """
      {"container_configs":{"app":{"default":true,"create":["app-create.sh"],"cleanup":["app-cleanup.sh"]}}}
      """
    And the repo-local overlay is trusted
    When I run quecto with arguments "config get --effective container_configs"
    Then the exit code should be 0
    And the printed JSON should have "shared.default" set to false
    And the printed JSON should have "app.default" set to true
