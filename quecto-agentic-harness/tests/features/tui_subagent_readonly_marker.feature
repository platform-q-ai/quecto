@done @tui
Feature: Read-only sub-agent observer marker in the left panel (#966)
  As a human operator driving workflows in the TUI
  I want read-only sub-agents to show an observer marker in the left panel
  So that I can tell at a glance which sub-agents cannot mutate the repository

  Scenario: Read-only sub-agents are visible as observers
    Given a sub-agent-first TUI tracking a read-only sub-agent "ro1"
    When the operator views the left sub-agent panel
    Then the left panel shows sub-agent "ro1" as an observer

  Scenario: Read-write sub-agents are not shown as observers
    Given a sub-agent-first TUI tracking a read-write sub-agent "rw1"
    When the operator views the left sub-agent panel
    Then the left panel shows sub-agent "rw1" without an observer marker

  Scenario: Observer status is kept with the correct sub-agent
    Given a sub-agent-first TUI tracking a read-only sub-agent "ro1" and a read-write sub-agent "rw1"
    When the operator views the left sub-agent panel
    Then the left panel shows sub-agent "ro1" as an observer
    And the left panel shows sub-agent "rw1" without an observer marker
    And only sub-agent "ro1" is shown as an observer

  Scenario: Observer status is removed when a sub-agent leaves
    Given a sub-agent-first TUI tracking a read-only sub-agent "ro1" and a read-write sub-agent "rw1"
    When sub-agent "ro1" leaves
    Then the left panel shows sub-agent "rw1" without an observer marker
    And the left panel no longer shows sub-agent "ro1"
    And the left panel shows no observer sub-agents
