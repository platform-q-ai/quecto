Feature: Container-backed subagent orchestration
  As a parent agent coordinating concurrent repository work
  I want subagents to run in named isolated environments
  So that independent work can proceed without sharing a checkout

  # Acceptance criteria (#1369)
  # AC1 Local spawning remains the default when no container is requested.
  # AC2 A new environment uses the explicitly selected script set, or the configured
  #     default; missing, unknown, and incomplete selections fail before creation.
  # AC3 New environments may use the parent repository or an explicit repository URL.
  # AC4 An existing live environment can receive another agent, including an observer;
  #     unknown or dead refs fail without guessing.
  # AC5 Environment refs are stable, session-scoped, visible to the parent, resolvable
  #     by protocol commands, and never reused.
  # AC6 Environment and agent identities remain distinct while agents in one
  #     environment share its checkout.
  # AC7 Script-managed agents retain the existing agent command, workflow, status,
  #     transcript, kill, await, and cleanup behaviour.
  # AC8 Script outputs and metadata are retained without runtime-specific inference.
  # AC9 Socket EOF pushes agent death, triggers one post-mortem inspection, and updates
  #     status and pending waits without polling.
  # AC10 The TUI shows a solo environment ref inline, groups two or more agents under a
  #      selectable environment row, and exposes environment details when selected.
  # AC11 The reference scripts satisfy the documented create, exec, inspect, and kill
  #      contract without embedding Docker assumptions in the agent runtime.

  @done
  Scenario: Local execution remains the default
    Given a parent agent is configured with container scripts
    When the parent spawns an agent without a container request
    Then the agent runs in the parent's local environment

  @done
  Scenario: The configured default deterministically creates an isolated environment
    Given a parent agent has a valid default container script set
    When the parent spawns an agent in a new container
    Then the agent runs in a newly registered isolated environment

  @pending
  Scenario: An explicit script selection overrides the default
    Given a parent agent has multiple valid container script sets
    When the parent spawns an agent in a new container with an explicit script selection
    Then the selected script set creates the isolated environment

  @pending
  Scenario Outline: Invalid script selection creates no environment
    Given a parent agent has a <configuration> container script selection
    When the parent spawns an agent in a new container
    Then the spawn fails before an environment is created

    Examples:
      | configuration |
      | missing       |
      | unknown       |
      | incomplete    |

  @pending
  Scenario: A new environment defaults to the parent repository
    Given a parent agent has a valid default container script set
    When the parent spawns an agent in a new container without a repository
    Then the isolated environment uses the parent's repository

  @pending
  Scenario: A new environment can use another repository
    Given a parent agent has a valid default container script set
    When the parent spawns an agent in a new container for an explicit repository
    Then the isolated environment uses the requested repository

  @done
  Scenario: An observer joins an existing environment
    Given a live isolated environment contains an implementing agent
    When the parent spawns a read-only agent into that environment
    Then both agents share the environment checkout

  @done
  Scenario Outline: An unavailable environment ref is never guessed
    Given an environment ref is <availability>
    When the parent spawns an agent into that environment ref
    Then the spawn fails without targeting another environment

    Examples:
      | availability |
      | unknown      |
      | dead         |

  @done
  Scenario: Environment refs are never reused
    Given an isolated environment has stopped
    When the parent creates another isolated environment
    Then the new environment has a later session ref

  @pending
  Scenario: The parent can discover environment membership
    Given multiple agents share an isolated environment
    When the parent lists its managed environments
    Then the environment listing identifies its ref, repository, and member agents

  @pending
  Scenario: Killing an environment cleans up its agents and runtime
    Given multiple agents share a live isolated environment
    When the parent kills that environment by ref
    Then its agents exit and its runtime resources are cleaned up

  @pending
  Scenario: Container-backed agents preserve the subagent protocol
    Given an agent runs in an isolated environment
    When the parent supervises the agent
    Then transcript, workflow, status, command, kill, and await behaviour matches a local agent

  @pending
  Scenario: Script-reported environment metadata remains available
    Given a script-managed environment reports runtime and workspace metadata
    When the parent inspects the environment
    Then the reported metadata is available without runtime-specific inference

  @pending
  Scenario: Agent death is pushed to supervision
    Given a running container-backed agent has a liveness connection
    When the agent socket closes
    Then the agent is marked exited after one environment post-mortem

  @pending
  Scenario: A solo container-backed agent stays flat in the panel
    Given one agent belongs to an isolated environment
    When the operator views the agent panel
    Then the agent row shows its environment ref inline

  @pending
  Scenario: Shared environments become selectable panel groups
    Given two agents belong to one isolated environment
    When the operator views the agent panel
    Then the agents appear beneath a selectable environment row

  @pending
  Scenario: Selecting an environment exposes its details
    Given the agent panel contains an isolated environment group
    When the operator selects the environment
    Then the main pane shows its repository, runtime, workspace, and health details

  @pending
  Scenario: The reference scripts implement the public runtime contract
    Given the in-repository container runtime scripts
    When each supported environment operation completes
    Then it emits the documented machine-readable result
