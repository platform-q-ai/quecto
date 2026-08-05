Feature: Script-managed subagent spawning
  As a parent agent
  I want to spawn one child into a new script-managed environment over a direct UDS endpoint
  So that local spawning remains the default while configured runtime scripts can own environment creation

  @done @container-spawn
  Scenario: Local spawning remains the default when container is omitted
    Given live subagent spawning is available
    When I spawn local subagent "local-default-slice1" with initial task "LOCAL_DEFAULT_MARKER"
    Then the spawn result should not be an error
    And child "local-default-slice1" should receive "LOCAL_DEFAULT_MARKER"
    And the spawn result should not include an environment reference

  @done @container-spawn
  Scenario: Container false preserves local spawning
    Given live subagent spawning is available
    When I spawn local subagent "local-false-slice1" with container disabled and task "LOCAL_FALSE_MARKER"
    Then the spawn result should not be an error
    And child "local-false-slice1" should receive "LOCAL_FALSE_MARKER"
    And the spawn result should not include an environment reference

  @done @container-spawn
  Scenario: Container true creates a default script-managed environment
    Given script-managed subagent spawning is available with default script "default"
    When I spawn script-managed subagent "container-default-slice1" with default selection and task "CONTAINER_DEFAULT_MARKER"
    Then the spawn result should not be an error
    And the spawn result should include an environment reference
    And the script-managed runtime should have used container script "default"
    And child "container-default-slice1" should receive "CONTAINER_DEFAULT_MARKER"

  @done @container-spawn
  Scenario: Explicit script selection overrides the default script
    Given script-managed subagent spawning is available with default script "default"
    When I spawn script-managed subagent "container-explicit-slice1" with script "alternate" and task "CONTAINER_EXPLICIT_MARKER"
    Then the spawn result should not be an error
    And the spawn result should include an environment reference
    And the script-managed runtime should have used container script "alternate"
    And child "container-explicit-slice1" should receive "CONTAINER_EXPLICIT_MARKER"

  @done @container-spawn
  Scenario Outline: Unknown container input is rejected before environment creation
    Given script-managed subagent spawning is available with default script "default"
    When I spawn subagent "container-unknown-field-slice1" with unsupported container field "<field>"
    Then the spawn result should fail because unsupported container field "<field>" is not allowed
    And the script-managed runtime should not have been invoked
    And the spawn result should not include an environment reference

    Examples:
      | field |
      | branch |
      | pr |
      | image |
      | runtime |

  @done @container-spawn
  Scenario: Existing container mode is rejected before environment creation
    Given script-managed subagent spawning is available with default script "default"
    When I spawn subagent "container-existing-slice1" into an existing container
    Then the spawn result should fail because existing containers are unsupported
    And the script-managed runtime should not have been invoked
    And the spawn result should not include an environment reference

  @done @container-spawn
  Scenario Outline: Invalid script configuration fails before environment creation
    Given script-managed subagent spawning has "<config_error>" runtime configuration
    When I spawn script-managed subagent "container-invalid-config-slice1" with default selection and task "NO_CREATE_MARKER"
    Then the spawn result should fail because script configuration "<config_error>" is invalid
    And the script-managed runtime should not have been invoked
    And the spawn result should not include an environment reference

    Examples:
      | config_error |
      | missing default |
      | default name not found |
      | missing create argv |
      | empty create argv |
      | unsafe create argv |
      | unknown config field |

  @done @container-spawn
  Scenario: Omitted repository uses the parent repository URL
    Given script-managed subagent spawning is available with parent repository "https://example.invalid/parent.git"
    When I spawn script-managed subagent "container-parent-repo-slice1" with default selection and task "PARENT_REPO_MARKER"
    Then the spawn result should not be an error
    And the script-managed runtime should have received repository "https://example.invalid/parent.git"
    And child "container-parent-repo-slice1" should receive "PARENT_REPO_MARKER"

  @done @container-spawn
  Scenario: Explicit repository is passed literally to the script-managed runtime
    Given script-managed subagent spawning is available with default script "default"
    When I spawn script-managed subagent "container-repo-slice1" for repository "https://example.invalid/repo.git" and task "REPO_MARKER"
    Then the spawn result should not be an error
    And the script-managed runtime should have received repository "https://example.invalid/repo.git"
    And child "container-repo-slice1" should receive "REPO_MARKER"

  @done @container-spawn
  Scenario: Script-managed creation starts exactly one child without local fallback
    Given script-managed subagent spawning is available with default script "default"
    When I spawn script-managed subagent "container-once-slice1" with default selection and task "EXACTLY_ONCE_MARKER"
    Then the spawn result should not be an error
    And the script-managed runtime should have started exactly 1 child
    And the spawn result should include an environment reference
    And no local fallback child should have been started
    And the script-managed runtime should have received the configured base directory
    And child "container-once-slice1" should be reachable

  @done @container-spawn
  Scenario: Direct UDS child accepts follow-up prompts
    Given script-managed child "container-protocol-slice1" is running with task "PROTOCOL_BOOTSTRAP_MARKER"
    When I send prompt "PROTOCOL_PROMPT_MARKER" to child "container-protocol-slice1"
    Then the agent command result should not be an error
    And child "container-protocol-slice1" should be reachable
    And child "container-protocol-slice1" should receive "PROTOCOL_PROMPT_MARKER"

  @done @container-spawn
  Scenario: Script-managed proxy endpoint is unsupported in this slice
    Given script-managed subagent spawning returns a proxy endpoint
    When I spawn script-managed subagent "container-proxy-slice1" with default selection and task "PROXY_MARKER"
    Then the spawn result should fail because proxy endpoints are unsupported
    And the script-managed runtime should have cleaned up exactly 1 environment
    And child "container-proxy-slice1" should not be reachable
    And the subagent registry should not contain "container-proxy-slice1"

  @done @container-spawn
  Scenario Outline: Rollback after script-managed transaction failure cleans up exactly once
    Given script-managed subagent spawning fails during "<phase>"
    When I spawn script-managed subagent "container-rollback-slice1" with default selection and task "ROLLBACK_MARKER"
    Then the spawn result should fail because script-managed launch failed during "<phase>"
    And the script-managed runtime should have cleaned up exactly 1 environment
    And the script-managed cleanup should target the created environment
    And child "container-rollback-slice1" should not be reachable
    And the subagent registry should not contain "container-rollback-slice1"

    Examples:
      | phase |
      | readiness |
      | initial prompt |
