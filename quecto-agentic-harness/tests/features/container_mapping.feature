@container-mapping
Feature: A repository binds itself to a container config through its overlay
  As an agent working in a checked-out repository
  I want `spawn` with `container: true` to select the container config the repository's trusted
  `.quecto/config.json` overlay labels as default, over the global file
  So that "this repo's containers use config X" is one small, trusted, agent-writable overlay

  Background:
    Given script-managed subagent spawning is available from a checkout with global default script "default"

  @done @issue-2024 @container-spawn
  Scenario: The trusted overlay's default config is selected by container true
    Given the checkout binds itself to container config "r" with repository "https://example.test/repo-r" through quecto config set --local
    When I spawn script-managed subagent "container-repo-r" with default selection and no config argument and task "CONTAINER_REPO_R_MARKER"
    Then the spawn result should not be an error
    And the spawn result should include an environment reference
    And the spawn result should name container config "r"
    And the spawn result should carry no configuration diagnostics
    And the script-managed runtime should have used container script "r"
    And the script-managed runtime should have received repository "https://example.test/repo-r"
    And child "container-repo-r" should receive "CONTAINER_REPO_R_MARKER"

  @done @issue-2024 @container-spawn @serial
  Scenario: A real quecto agent started in the bound checkout spawns its overlay's container
    Given the checkout binds itself to container config "r" with repository "https://example.test/repo-r" through quecto config set --local
    When a real quecto agent started in the checkout is driven by a fake provider to spawn container true
    Then the real agent should have exited successfully
    And the script-managed runtime should have used container script "r"
    And the script-managed runtime should have received repository "https://example.test/repo-r"
    And the tool result the fake provider received should name container config "r"

  @done @issue-2024 @container-spawn
  Scenario: A checkout without an overlay keeps the global default
    When I spawn script-managed subagent "container-no-overlay" with default selection and no config argument and task "CONTAINER_NO_OVERLAY_MARKER"
    Then the spawn result should not be an error
    And the script-managed runtime should have used container script "default"
    And the script-managed runtime should have received no repository

  @done @issue-2024 @container-spawn
  Scenario: An untrusted overlay refuses container true with the configuration diagnostic in the tool result
    Given the checkout carries an untrusted overlay binding container config "r" with repository "https://example.test/repo-r"
    When I spawn script-managed subagent "container-untrusted" with default selection and no config argument and task "CONTAINER_UNTRUSTED_MARKER"
    Then the spawn result should fail with "container: true refused: the checkout's repo-local config overlay was not applied"
    And the spawn result should carry the configuration diagnostic naming the checkout's overlay
    And the script-managed runtime should not have been invoked

  @done @issue-2024 @container-spawn
  Scenario: A named global config launches over an untrusted overlay with the configuration diagnostic in the tool result
    Given the checkout carries an untrusted overlay binding container config "r" with repository "https://example.test/repo-r"
    When I spawn script-managed subagent "container-untrusted-named" with script "alternate" and no config argument and task "CONTAINER_UNTRUSTED_NAMED_MARKER"
    Then the spawn result should not be an error
    And the spawn result should name container config "alternate"
    And the spawn result should carry the configuration diagnostic naming the checkout's overlay
    And the script-managed runtime should have used container script "alternate"
    And the script-managed runtime should have received no repository

  @done @issue-2024 @container-spawn
  Scenario: A name only the untrusted overlay defines is unknown and the result names the withheld overlay
    Given the checkout carries an untrusted overlay binding container config "r" with repository "https://example.test/repo-r"
    When I spawn script-managed subagent "container-untrusted-overlay-only" with script "r" and no config argument and task "CONTAINER_UNTRUSTED_OVERLAY_ONLY_MARKER"
    Then the spawn result should fail with "unknown container config 'r' (available container configs: alternate, default)"
    And the spawn result should carry the configuration diagnostic naming the checkout's overlay
    And the script-managed runtime should not have been invoked

  @done @issue-2024 @container-spawn
  Scenario: An untrusted overlay that cannot have changed the container set warns and keeps the global default
    Given the checkout carries an untrusted overlay pinning only the default model "pinned-model"
    When I spawn script-managed subagent "container-untrusted-model-only" with default selection and no config argument and task "CONTAINER_UNTRUSTED_MODEL_ONLY_MARKER"
    Then the spawn result should not be an error
    And the spawn result should name container config "default"
    And the spawn result should carry the configuration diagnostic naming the checkout's overlay
    And the script-managed runtime should have used container script "default"
    And the script-managed runtime should have received no repository

  @done @issue-2024 @container-spawn
  Scenario: A trusted overlay whose merge is invalid fails the spawn naming the overlay before any script runs
    Given the checkout carries a trusted overlay labelling both "r1" and "r2" as default container configs
    When I spawn script-managed subagent "container-invalid-merge" with default selection and no config argument and task "CONTAINER_INVALID_MERGE_MARKER"
    Then the spawn result should fail naming the checkout's overlay merged over the global file
    And the spawn result should fail with "multiple container configs are labeled"
    And the spawn result should fail with "(r1, r2); exactly one is allowed"
    And the script-managed runtime should not have been invoked

  @done @issue-2024 @container-spawn
  Scenario: A non-default overlay entry is selectable by name and the global default stays
    Given the checkout adds non-default container config "r" with repository "https://example.test/repo-r" through quecto config set --local
    When I spawn script-managed subagent "container-named-r" with script "r" and no config argument and task "CONTAINER_NAMED_R_MARKER"
    Then the spawn result should not be an error
    And the script-managed runtime should have used container script "r"
    And the script-managed runtime should have received repository "https://example.test/repo-r"

  @done @issue-2024 @container-spawn
  Scenario: Selection errors enumerate the merged set of global and overlay configs
    Given the checkout adds non-default container config "r" with repository "https://example.test/repo-r" through quecto config set --local
    When I spawn script-managed subagent "container-unknown" with script "nope" and no config argument and task "CONTAINER_UNKNOWN_MARKER"
    Then the spawn result should fail with "unknown container config 'nope' (available container configs: alternate, default, r)"
    And the script-managed runtime should not have been invoked

  @done @issue-2024 @container-spawn
  Scenario: An explicit config argument replaces the layered configuration and ignores the overlay
    Given the checkout binds itself to container config "r" with repository "https://example.test/repo-r" through quecto config set --local
    When I spawn script-managed subagent "container-explicit" with default selection and task "CONTAINER_EXPLICIT_MARKER"
    Then the spawn result should not be an error
    And the script-managed runtime should have used container script "default"
    And the script-managed runtime should have received no repository

  @done @issue-2024 @container-spawn
  Scenario: Unsetting the overlay entry rolls the checkout back to the global default
    Given the checkout binds itself to container config "r" with repository "https://example.test/repo-r" through quecto config set --local
    And the checkout unbinds container config "r" through quecto config unset --local
    When I spawn script-managed subagent "container-rollback" with default selection and no config argument and task "CONTAINER_ROLLBACK_MARKER"
    Then the spawn result should not be an error
    And the script-managed runtime should have used container script "default"
    And the script-managed runtime should have received no repository
