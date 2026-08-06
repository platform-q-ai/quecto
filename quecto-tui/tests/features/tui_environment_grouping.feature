@tui @env-grouping
Feature: Environment visibility and grouping in the sub-agent panel (#1369 slice 4)
  As a human operator coordinating container-backed sub-agents from one TUI
  I want isolated PR environments distinguished in the left panel — solo agents
  compact with a dim environment badge, shared environments grouped under one
  selectable environment row — and the selected environment's details in the
  main pane
  So that I can understand the environment layout of the session at a glance

  # Wired to step definitions in tests/bdd/tui_environment_grouping_steps.rs,
  # which drive the REAL render path through the headless render harness
  # (quecto_tui::shell::app::tui_harness) and the real wire deserializer via
  # event_line — no hard-coded output strings. Also covered by the unit tests in
  # quecto-tui/src/agents/app_subagent_environment_tests.rs.

  @done
  Scenario: A solo script-managed agent renders as a flat row with a dim environment badge
    Given a TUI on a 120-column terminal tracking sub-agent "impl" running alone in environment "C1"
    Then the panel row for "impl" shows the environment badge "C1" between the tree stalk and the name
    And the panel renders no separate environment row for "C1"

  @done
  Scenario: The solo environment badge survives narrow-panel truncation
    Given a TUI on a 48-column terminal tracking sub-agent "implementer-having-a-very-long-name" running alone in environment "C1"
    Then the panel row keeps the "C1" badge
    And the agent name is truncated within the clamped panel width

  @done
  Scenario: Two agents sharing an environment group under one selectable environment row
    Given a TUI on a 120-column terminal tracking sub-agents "impl" and "rev" sharing environment "C2"
    Then the panel shows one environment row for "C2"
    And the agents "impl" and "rev" are nested beneath the "C2" environment row with tree connectors
    And no duplicate root rows are rendered for "impl" or "rev"

  @done
  Scenario: Selecting an environment row shows environment details in the main pane
    Given a TUI on a 120-column terminal tracking sub-agents "impl" and "rev" sharing environment "C2"
    When I select the environment row "C2" through panel navigation
    Then the main pane shows environment details for "C2" including name, status, repository, branch, runtime id, workspace and socket mode

  @done
  Scenario: The solo environment badge survives a sparse roster refresh
    Given a TUI on a 120-column terminal tracking sub-agent "impl" running alone in environment "C1"
    When a sparse get_subagents roster refresh omits the environment metadata for "impl"
    Then the panel row for "impl" still shows the environment badge "C1"

  @done
  Scenario: Environment details survive a live update followed by a sparse snapshot refresh
    Given a TUI on a 120-column terminal tracking sub-agents "impl" and "rev" sharing environment "C2"
    When a sparse get_subagents roster refresh omits the environment metadata for "impl" and "rev"
    And I select the environment row "C2" through panel navigation
    Then the main pane shows environment details for "C2" including name, status, repository, branch, runtime id, workspace and socket mode

  @done
  Scenario: A local-only session renders without environment chrome
    Given a TUI on a 120-column terminal tracking local-only sub-agent "solo"
    Then the panel contains no environment badge or environment row

  @done
  Scenario: The fixed panel width is 34 columns on a wide terminal
    Given a TUI on a 120-column terminal tracking local-only sub-agent "solo"
    Then the left panel is rendered 34 columns wide
