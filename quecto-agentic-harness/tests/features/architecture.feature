@done @architecture
Feature: Architecture boundaries and ports
  As a maintainer
  I want architecture contracts encoded in executable tests
  So that boundary regressions are caught before refactors land

  Scenario: Application layer avoids direct runtime I/O
    Then the application source should not contain runtime I/O patterns

  Scenario: Authoritative CI lints all workspace members
    Then authoritative CI should lint with --workspace flag

  Scenario: Authoritative CI runs the zero-cost mocked e2e suite
    Then authoritative CI should run the mocked e2e suite

  Scenario: Pre-push never selects a paid real-LLM lane from provider credentials
    Then the pre-push script should not probe for a provider key to auto-run the paid suite

  Scenario: The mocked e2e suite covers the curated real-LLM capability checklist
    Then the mocked e2e suite should cover the curated real-LLM capability checklist

  Scenario: Retired live behavioral e2e scenarios are manual-only
    Then the retired live behavioral e2e suite should be tagged manual-only

  Scenario: Provider smoke scenarios stay out of automocked lanes
    Then provider smoke scenarios should not be tagged as mocked or manual real LLM

  @dependency-hygiene
  Scenario: Retired tool-installer support stays out of normal harness builds
    Given the harness normal build configuration is inspected
    When retired installer support is classified
    Then normal builds should exclude the retired installer
    And normal builds should exclude its archive dependencies

  @dependency-hygiene
  Scenario: Search tools explain how to install missing binaries
    Given the harness search tools are inspected
    When their missing-binary handling is checked
    Then each search tool should keep direct install guidance

  @dependency-hygiene
  Scenario: Text normalization is scoped to macOS builds
    Given the harness dependency manifest is inspected
    When platform-specific dependencies are classified
    Then text normalization should be scoped to macOS builds
