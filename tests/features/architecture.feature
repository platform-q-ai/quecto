@wip @architecture
Feature: Architecture boundaries and ports
  As a maintainer
  I want architecture contracts encoded in executable tests
  So that boundary regressions are caught before refactors land

  Scenario: Application layer avoids direct runtime I/O
    Then the application source should not contain runtime I/O patterns

  Scenario: Pre-push clippy lints all workspace members
    Then the pre-push script should lint with --workspace flag

  Scenario: Pre-push runs the zero-cost mocked e2e suite by default
    Then the pre-push script should run the mocked e2e suite by default

  Scenario: Pre-push no longer auto-enables the paid real-LLM suite from a key
    Then the pre-push script should not probe for a provider key to auto-run the paid suite

  Scenario: Pre-push exposes a documented opt-in for the live real-LLM suite
    Then the pre-push script should gate the live real-LLM suite behind an explicit opt-in flag

  Scenario: The mocked e2e suite preserves real-LLM behavioural coverage
    Then the mocked e2e suite should preserve the real-LLM behavioural coverage
