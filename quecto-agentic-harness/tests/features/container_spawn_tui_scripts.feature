Feature: Container runtime TUI grouping and script contract

  # AC10 clause 1
  @done @issue-1369
  Scenario: Solo container-backed agent rows expose environment refs inline
    Given a hybrid container panel check
    When the panel contains one solo environment and one shared environment
    Then the solo agent row exposes its environment ref inline

  # AC10 clause 2
  @done @issue-1369
  Scenario: Shared environments render as selectable rows with grouped agents beneath
    Given a hybrid container panel check
    When the panel contains one solo environment and one shared environment
    Then the shared environment is exposed as a selectable group row with its member agents beneath it

  # AC10 clause 3
  @done @issue-1369
  Scenario: Selecting a shared environment renders environment details in the main pane
    Given a hybrid container panel check
    When the shared environment row is selected
    Then the main pane renders the selected environment repository
    And the main pane renders the selected environment runtime
    And the main pane renders the selected environment workspace
    And the main pane renders the selected environment health

  # AC11
  @done @issue-1369
  Scenario: Reference container runtime scripts and documentation are present
    Given a container runtime contract check
    When I check supported runtime operations
    Then the create container runtime script is present and executable
    And the exec container runtime script is present and executable
    And the inspect container runtime script is present and executable
    And the kill container runtime script is present and executable
    And docs/container-runtimes.md documents the create exec inspect kill contract

  # AC11
  @done @issue-1369
  Scenario: Reference container runtime scripts declare machine-readable outputs
    Given a container runtime contract check
    When I check supported runtime operations
    Then each container runtime script documents a JSON result contract
    And docs/container-runtimes.md documents required JSON fields for each operation
