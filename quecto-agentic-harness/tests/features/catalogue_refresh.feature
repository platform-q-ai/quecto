@catalogue-refresh
Feature: Discovery as refreshable catalogue source with one refresh use case (epic #1193, slice 4)
  As a maintainer of the provider/model catalogue
  I want model discovery to be a refreshable catalogue source driven by one refresh use case
  So that refreshes report per-source outcomes and publish through the normal catalogue path

  @wip
  Scenario: Refreshing all sources reports per-source outcomes and publishes a new generation
    Given a refreshable catalogue source "openrouter" that will report models "alpha" and "beta"
    And a refreshable catalogue source "local" that will report no change
    And the effective catalogue has been resolved and published for refresh
    When all catalogue sources are refreshed
    Then the refresh outcome for "openrouter" is updated with 2 models
    And the refresh outcome for "local" is unchanged
    And the refresh publishes catalogue generation 2

  @wip
  Scenario: Refreshing a selected subset touches only the named sources
    Given a refreshable catalogue source "openrouter" that will report models "alpha" and "beta"
    And a refreshable catalogue source "local" that will report no change
    When only catalogue source "openrouter" is refreshed
    Then the refresh outcome for "openrouter" is updated with 2 models
    And source "local" was never asked to refresh

  @wip
  Scenario: An unsupported provider reports an actionable unsupported outcome
    Given a refreshable catalogue source "openrouter" that will report models "alpha" and "beta"
    And a catalogue source "anthropic" that does not support remote refresh because "provider does not expose a model listing endpoint"
    When all catalogue sources are refreshed
    Then the refresh outcome for "anthropic" is unsupported mentioning "model listing endpoint"
    And the refresh outcome for "openrouter" is updated with 2 models

  @wip
  Scenario: One failing source never discards other sources' successes
    Given a refreshable catalogue source "openrouter" that will report models "alpha" and "beta"
    And a refreshable catalogue source "flaky" that will fail with "connection refused"
    When all catalogue sources are refreshed
    Then the refresh outcome for "openrouter" is updated with 2 models
    And the refresh outcome for "flaky" is failed mentioning "connection refused"
    And the published snapshot contains model "openrouter/alpha"

  @wip
  Scenario: Total refresh failure retains the previous valid catalogue
    Given a refreshable catalogue source "flaky" that will fail with "connection refused"
    And the effective catalogue has been resolved and published for refresh
    When all catalogue sources are refreshed
    Then the refresh outcome for "flaky" is failed mentioning "connection refused"
    And the previously published catalogue generation is retained

  @wip
  Scenario: Cancellation preserves completed successes and the previous valid state
    Given a refreshable catalogue source "openrouter" that will report models "alpha" and "beta"
    And a refreshable catalogue source "slow" whose refresh triggers cancellation
    And the effective catalogue has been resolved and published for refresh
    When all catalogue sources are refreshed
    Then the refresh outcome for "openrouter" is updated with 2 models
    And the refresh outcome for "slow" is cancelled
    And the published snapshot contains model "openrouter/alpha"

  @wip
  Scenario: Refreshes are bounded for unattended use
    Given a refreshable catalogue source "openrouter" that will report models "alpha" and "beta"
    When all catalogue sources are refreshed with a timeout of 5 seconds and a response cap of 1048576 bytes
    Then source "openrouter" observed a refresh timeout of 5 seconds
    And source "openrouter" observed a refresh response cap of 1048576 bytes

  @wip
  Scenario: A user override still wins over refreshed discovered data
    Given a refreshable catalogue source "openrouter" that will report models "alpha" and "beta"
    And a user-override catalogue source "user" naming model "openrouter/alpha" "My Alpha"
    When all catalogue sources are refreshed
    Then the refresh-published model "openrouter/alpha" is named "My Alpha"

  @wip
  Scenario: Refresh outcomes never contain credential material
    Given a refreshable catalogue source "flaky" that will fail with "401 unauthorized for bearer sk-refresh-secret-123"
    And the refresh credential secret is "sk-refresh-secret-123"
    When all catalogue sources are refreshed
    Then no refresh outcome contains the secret "sk-refresh-secret-123"
