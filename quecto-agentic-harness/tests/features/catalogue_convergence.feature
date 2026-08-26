@catalogue-convergence
Feature: Legacy authority removal and cross-surface convergence (epic #1193, slice 6)
  As a Quecto maintainer
  I want every remaining legacy provider/model authority removed or subordinated
  So that the application-published catalogue snapshot is the only source of truth

  # AC1 — no catalogue composition or parsing in CLI-specific interface modules
  @done
  Scenario: Interface layer owns no catalogue composition bridges
    Given the harness source tree
    When the source tree is scanned for legacy authorities
    Then the CLI interface declares no catalogue bridge modules
    And no interface module reads the legacy model registry
    And no infrastructure module defines canonical catalogue types

  # AC1 — capability heuristics move into canonical metadata
  @done
  Scenario: Effort capability lives in canonical catalogue metadata
    Given the harness source tree
    When the source tree is scanned for legacy authorities
    Then canonical model capabilities declare an effort vocabulary
    And no interface or infrastructure module infers effort levels from model names

  # AC2 — cross-surface conformance: the listing surfaces project canonical
  # capability metadata rather than re-deriving it per surface
  @done
  Scenario: Listing surfaces publish the snapshot's effort vocabulary
    Given a base directory with only built-in catalogue data
    When the model listing is requested
    Then every listed model carries an effort vocabulary from the snapshot
    And the listed model "anthropic-api/claude-opus-4-6" has effort vocabulary "low, medium, high, max"
    And the listed model "openai-api/gpt-5.5" has effort vocabulary "none, low, medium, high, xhigh"

  # AC4 — contributor documentation
  @done
  Scenario: Contributor documentation maps layer ownership and forbids new authorities
    Given the contributor documentation exists
    When a contributor reads it
    Then it maps layer ownership across domain, application, infrastructure, and interface
    And it explains how to add domain metadata
    And it explains how to add a catalogue source
    And it explains user overrides
    And it warns against creating another authority
