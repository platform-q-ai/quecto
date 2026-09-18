@tui @done @setup
Feature: /setup submits the agent-executable setup walkthrough (#2024 S6)
  As a TUI user on a fresh folder or machine
  I want /setup to hand the parent agent the setup walkthrough prompt
  So that the agent reads the `setup` docs page and proposes each step, asking before it writes

  Background:
    Given a fresh TUI app harness

  Scenario: /setup submits the all-areas walkthrough as a visible user turn
    When I submit the master prompt "/setup"
    Then the master transcript shows the setup walkthrough prompt for "all areas" as the user's turn
    And a prompt command is sent carrying the setup walkthrough prompt for "all areas"
    And the setup walkthrough prompt for "all areas" names the docs page "setup"

  Scenario: /setup model pins a model through the models runbook
    When I submit the master prompt "/setup model openai-api/gpt-5.5"
    Then the master transcript shows the setup walkthrough prompt for "model openai-api/gpt-5.5" as the user's turn
    And a prompt command is sent carrying the setup walkthrough prompt for "model openai-api/gpt-5.5"
    And the setup walkthrough prompt for "model openai-api/gpt-5.5" names the docs page "models"

  Scenario Outline: area variants name their runbook page
    When I submit the master prompt "/setup <variant>"
    Then a prompt command is sent carrying the setup walkthrough prompt for "<variant>"
    And the setup walkthrough prompt for "<variant>" names the docs page "<page>"

    Examples:
      | variant   | page              |
      | admission | admission-broker  |
      | podman    | container-runtime |
      | container | container-runtime |
      | auth      | models            |

  Scenario: an unknown variant shows the usage toast and submits nothing
    When I submit the master prompt "/setup frobnicate" expecting no agent command
    Then a setup usage toast lists the variants
    And no prompt command is sent
    And the master transcript has no user turn

  Scenario: /setup model without a model id shows the usage toast
    When I submit the master prompt "/setup model" expecting no agent command
    Then a setup usage toast lists the variants
    And no prompt command is sent

  Scenario: every setup prompt is safe by construction
    Then every setup walkthrough prompt asks before writing, forbids secrets and dry-runs the service install

  Scenario: /help lists /setup and it autocompletes
    Then the help listing shows "/setup"
    And the slash-command autocomplete offers "setup"
