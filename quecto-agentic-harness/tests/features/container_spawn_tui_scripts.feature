Feature: Container runtime TUI grouping and script contract
  As an operator using container-backed subagents
  I want the panel and reference runtime scripts to expose the public contract
  So that AC10 and AC11 are pinned before implementation

  # AC10
  @done
  Scenario: Solo container-backed agents render flat while shared environments render as groups
    Given a container runtime contract check
    When the panel contains one solo environment and one shared environment
    Then the solo agent row exposes its environment ref inline
    And the shared environment is exposed as a selectable group row with its member agents beneath it

  # AC11
  @done
  Scenario: Reference container runtime scripts and documentation are present
    Given a container runtime contract check
    When I check supported runtime operations
    Then the create container runtime script is present and executable
    And the exec container runtime script is present and executable
    And the inspect container runtime script is present and executable
    And the kill container runtime script is present and executable
    And docs/container-runtimes.md documents the create exec inspect kill contract

  # AC11
  @done
  Scenario: Reference container runtime scripts declare machine-readable outputs
    Given a container runtime contract check
    When I check supported runtime operations
    Then each container runtime script documents a JSON result contract
    And docs/container-runtimes.md documents required JSON fields for each operation
