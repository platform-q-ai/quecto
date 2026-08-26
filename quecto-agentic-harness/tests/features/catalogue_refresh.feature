@catalogue-refresh
Feature: Discovery as refreshable catalogue source with one refresh use case (epic #1193, slice 4)
  As a maintainer of the provider/model catalogue
  I want model discovery to be a refreshable catalogue source driven by one refresh use case
  So that refreshes report per-source outcomes and publish through the normal catalogue path

  @done
  Scenario: Refreshing all sources reports a per-source outcome for each source
    Given a refreshable catalogue source "openrouter" that will report models "alpha" and "beta"
    And a refreshable catalogue source "local" that will report no change
    When all catalogue sources are refreshed
    Then the refresh outcome for "openrouter" is updated with 2 models
    And the refresh outcome for "local" is unchanged

  @done
  Scenario: A successful refresh publishes a new catalogue generation
    Given a refreshable catalogue source "openrouter" that will report models "alpha" and "beta"
    And the effective catalogue has been resolved and published for refresh
    When all catalogue sources are refreshed
    Then a new catalogue generation is published
    And the published snapshot contains model "openrouter/alpha"

  @done
  Scenario: Refreshing a selected subset touches only the named sources
    Given a refreshable catalogue source "openrouter" that will report models "alpha" and "beta"
    And a refreshable catalogue source "local" that will report no change
    When only catalogue source "openrouter" is refreshed
    Then the refresh outcome for "openrouter" is updated with 2 models
    And source "local" was never asked to refresh

  @done
  Scenario: An unsupported provider reports an actionable unsupported outcome
    Given a refreshable catalogue source "openrouter" that will report models "alpha" and "beta"
    And a catalogue source "anthropic" that does not support remote refresh because "provider does not expose a model listing endpoint"
    When all catalogue sources are refreshed
    Then the refresh outcome for "anthropic" is unsupported mentioning "model listing endpoint"
    And the refresh outcome for "openrouter" is updated with 2 models

  @done
  Scenario: One failing source never discards other sources' successes
    Given a refreshable catalogue source "openrouter" that will report models "alpha" and "beta"
    And a refreshable catalogue source "flaky" that will fail with "connection refused"
    And the effective catalogue has been resolved and published for refresh
    When all catalogue sources are refreshed
    Then the refresh outcome for "openrouter" is updated with 2 models
    And the refresh outcome for "flaky" is failed mentioning "connection refused"
    And the published snapshot contains model "openrouter/alpha"

  @done
  Scenario: Total refresh failure retains the previous valid catalogue
    Given a refreshable catalogue source "flaky" that will fail with "connection refused"
    And a user-override catalogue source "seed" naming model "seed/model" "Seed"
    And the effective catalogue has been resolved and published for refresh
    When all catalogue sources are refreshed
    Then the refresh outcome for "flaky" is failed mentioning "connection refused"
    And the previously published catalogue generation is retained
    And the published snapshot contains model "seed/model"

  @done
  Scenario: Cancellation preserves completed successes and the previous valid state
    Given a refreshable catalogue source "openrouter" that will report models "alpha" and "beta"
    And a refreshable catalogue source "slow" that is still refreshing when the refresh is cancelled
    And the effective catalogue has been resolved and published for refresh
    When all catalogue sources are refreshed
    Then the refresh outcome for "openrouter" is updated with 2 models
    And the refresh outcome for "slow" is cancelled
    And the published snapshot contains model "openrouter/alpha"

  @done
  Scenario: A refresh that outlives the timeout is reported failed
    Given a refresh timeout of 100 milliseconds
    And a refreshable catalogue source "slow" whose refresh outlives the refresh timeout
    When all catalogue sources are refreshed
    Then the refresh outcome for "slow" is failed mentioning "timeout"

  @done
  Scenario: A user override still wins over refreshed discovered data
    Given a refreshable catalogue source "openrouter" that will report models "alpha" and "beta"
    And a user-override catalogue source "user" naming model "openrouter/alpha" "My Alpha"
    When all catalogue sources are refreshed
    Then the refresh-published model "openrouter/alpha" is named "My Alpha"

  @done
  Scenario: Refresh outcomes never contain credential material
    Given a refreshable catalogue source "flaky" whose refresh fails with an error containing the credential secret "sk-refresh-secret-123"
    When all catalogue sources are refreshed
    Then the refresh outcome for "flaky" is failed mentioning "401 unauthorized"
    And no refresh outcome contains the secret "sk-refresh-secret-123"
