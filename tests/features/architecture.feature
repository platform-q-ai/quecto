@wip @architecture
Feature: Architecture boundaries and ports
  As a maintainer
  I want architecture contracts encoded in executable tests
  So that boundary regressions are caught before refactors land

  Scenario: Application layer avoids direct runtime I/O
    Then the application source should not contain runtime I/O patterns

  Scenario: Pre-push clippy lints all workspace members
    Then the pre-push script should lint with --workspace flag
