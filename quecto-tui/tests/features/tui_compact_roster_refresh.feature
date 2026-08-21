@tui @compact-roster
Feature: Compact get_subagents refresh keeps the left panel stable
  As a human operator coordinating live sub-agents from one TUI
  I want compact get_subagents polls after spawn and agent_cmd to keep
    already-visible children in the left panel
  So that a slim roster payload cannot flicker or wipe agents that are
    still answering on their sockets

  # Wired to step definitions in tests/bdd/tui_compact_roster_refresh_steps.rs.
  # Drives the REAL render path through the headless harness and the real
  # get_subagents response handler — no hard-coded output strings.

  @done
  Scenario: A compact get_subagents poll keeps a live child visible
    Given a TUI on a 120-column terminal tracking sub-agent "impl" running alone in environment "C1"
    When a compact get_subagents roster refresh reports "impl" as running in environment "C1"
    Then the panel shows one environment row for "C1"
    And the agent "impl" is nested beneath the "C1" environment row with the last-child connector
    And the agent "impl" appears exactly once, beneath the environment row

  @done
  Scenario: An unchanged compact get_subagents poll does not wipe the panel
    Given a TUI on a 120-column terminal tracking sub-agent "impl" running alone in environment "C1"
    When an unchanged compact get_subagents roster refresh arrives
    Then the panel shows one environment row for "C1"
    And the agent "impl" is nested beneath the "C1" environment row with the last-child connector
