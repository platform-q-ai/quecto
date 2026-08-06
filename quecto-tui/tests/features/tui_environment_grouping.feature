@tui @env-grouping
Feature: Environment visibility and grouping in the sub-agent panel (#1369 slice 4, follow-up revision)
  As a human operator coordinating container-backed sub-agents from one TUI
  I want every script-managed environment — one member or many — rendered as a
  selectable CN environment row with its member agents nested beneath it, and
  the selected environment's container information shown alone in the main pane
  So that container data is always reachable through the CN row and never reads
  as if a conversation belongs to the container

  # Wired to step definitions in tests/bdd/tui_environment_grouping_steps.rs,
  # which drive the REAL render path through the headless render harness
  # (quecto_tui::shell::app::tui_harness) and the real wire deserializer via
  # event_line — no hard-coded output strings. Also covered by the unit tests in
  # quecto-tui/src/agents/app_subagent_environment_tests.rs.

  @done
  Scenario: A solo script-managed agent renders as a full environment group
    Given a TUI on a 120-column terminal tracking sub-agent "impl" running alone in environment "C1"
    Then the panel shows one environment row for "C1"
    And the agent "impl" is nested beneath the "C1" environment row with the last-child connector
    And the agent "impl" appears exactly once, beneath the environment row

  @done
  Scenario: The solo environment group survives narrow-panel truncation
    Given a TUI on a 48-column terminal tracking sub-agent "implementer-having-a-very-long-name" running alone in environment "C1"
    Then the panel shows one environment row for "C1"
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
  Scenario: Selecting an environment row shows container information only
    Given a TUI on a 120-column terminal tracking sub-agents "impl" and "rev" sharing environment "C2"
    And a parent conversation is on screen
    When I select the environment row "C2" through panel navigation
    Then the main pane carries a container-info header and lists the members "impl" and "rev"
    And the main pane does not render the parent conversation
    When I select the master row through panel navigation
    Then the main pane renders the parent conversation again

  @done
  Scenario: Selecting a solo environment row shows its container information
    Given a TUI on a 120-column terminal tracking sub-agent "impl" running alone in environment "C1"
    When I select the environment row "C1" through panel navigation
    Then the main pane shows environment details for "C1" including name, status, repository, branch, runtime id, workspace and socket mode

  @done
  Scenario: The environment group survives a sparse roster refresh
    Given a TUI on a 120-column terminal tracking sub-agent "impl" running alone in environment "C1"
    When a sparse get_subagents roster refresh omits the environment metadata for "impl"
    Then the panel shows one environment row for "C1"

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
