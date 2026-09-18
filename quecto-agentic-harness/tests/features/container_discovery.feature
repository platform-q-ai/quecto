@container-discovery
Feature: Agents can discover which container configs and environments exist
  As an agent asked to "run a subagent in this repo's container"
  I want the spawn tool description to carry the effective container-config roster of my checkout,
  `agent_cmd get_container_configs` to return that set with each entry's source and repository,
  and the spawn and swarm descriptions to say what a new container really is
  So that my first spawn call succeeds instead of guessing a name and reading it from the error

  Background:
    Given script-managed subagent spawning is available from a checkout with global default script "default"

  @done @issue-2024 @container-spawn
  Scenario: The spawn description the model receives carries the effective roster of the checkout
    Given the checkout binds itself to container config "r" with repository "https://example.test/repo-r" through quecto config set --local
    When the launcher is composed for the checkout with container discovery
    Then the spawn description should carry the roster line "Available container configs: r (default, repo-bound), alternate (global), default (global)."
    And the spawn description roster line should be at most 120 characters

  @done @issue-2024 @container-spawn
  Scenario: A checkout without an overlay advertises the global set with its default first
    When the launcher is composed for the checkout with container discovery
    Then the spawn description should carry the roster line "Available container configs: default (default, global), alternate (global)."

  @done @issue-2024 @container-spawn
  Scenario: A withheld overlay is named in the roster line so the agent knows why container true is refused
    Given the checkout carries an untrusted overlay binding container config "r" with repository "https://example.test/repo-r"
    When the launcher is composed for the checkout with container discovery
    Then the spawn description should carry the roster line "Available container configs: alternate (global), default (global) (repo overlay untrusted — run quecto config trust)."

  @done @issue-2024 @container-spawn
  Scenario: A long roster is truncated with a count rather than overflowing the description
    Given the checkout adds 12 non-default container configs named "many" through quecto config set --local
    When the launcher is composed for the checkout with container discovery
    Then the spawn description roster line should be at most 120 characters
    And the spawn description roster line should end with "more."
    And the spawn description roster line should name "default (default, global)"

  @done @issue-2024 @container-spawn
  Scenario: get_container_configs returns the effective set with each entry's source and repository
    Given the checkout binds itself to container config "r" with repository "https://example.test/repo-r" through quecto config set --local
    When the launcher is composed for the checkout with container discovery
    And I run agent_cmd get_container_configs
    Then the container config listing should not be an error
    And the container config listing should have entry "r" with default true, source "overlay" and repository "https://example.test/repo-r"
    And the container config listing should have entry "default" with default false, source "global" and no repository
    And the container config listing should have entry "alternate" with default false, source "global" and no repository
    And the container config listing should report overlay_withheld false with no diagnostics

  @done @issue-2024 @container-spawn
  Scenario: get_container_configs reports a withheld overlay with the trust diagnostic
    Given the checkout carries an untrusted overlay binding container config "r" with repository "https://example.test/repo-r"
    When the launcher is composed for the checkout with container discovery
    And I run agent_cmd get_container_configs
    Then the container config listing should not be an error
    And the container config listing should have entry "default" with default false, source "global" and no repository
    And the container config listing should not have entry "r"
    And the container config listing should report overlay_withheld true with a diagnostic naming the checkout's overlay

  @done @issue-2024 @container-spawn
  Scenario: get_container_configs is refused when no configuration was composed for the session
    Given an AgentCmdTool with an empty registry
    When I run agent_cmd get_container_configs
    Then the container config listing should fail with "container config listing is not available in this session"

  @done @issue-2024 @container-spawn
  Scenario: The spawn and swarm descriptions tell the truth about what a container is
    When the launcher is composed for the checkout with container discovery
    Then the spawn description should say "fresh clone of the config's --repo at its default branch"
    And the spawn description should say "parent's working tree, branch and uncommitted changes are NOT inside"
    And the spawn description should say "push it and tell the child to fetch/checkout"
    And the spawn description should say "refs come from agent_cmd get_containers"
    And the spawn description should say "quecto container doctor"
    And the spawn description should say "agent_cmd get_container_configs"
    And the swarm description should say "the container the coordinator was spawned into"
    And the swarm description should say "workers with container omitted"
    And the swarm description should say "host-local reference scripts cannot host a swarm"
    And the agent_cmd description should say "get_container_configs"

  @done @issue-2024 @container-spawn @serial
  Scenario: A fresh real agent asked to run a subagent in this repo's container succeeds on its first spawn
    Given the checkout binds itself to container config "r" with repository "https://example.test/repo-r" through quecto config set --local
    When a real quecto agent started in the checkout is asked by a fake provider to run a subagent in this repo's container
    Then the real agent should have exited successfully
    And the spawn description the fake provider received should carry "Available container configs: r (default, repo-bound)"
    And the fake provider's get_container_configs result should list entry "r" from source "overlay"
    And the tool results the fake provider received should answer get_container_configs before the spawn
    And the tool result the fake provider received should name container config "r"
    And the script-managed runtime should have used container script "r"
