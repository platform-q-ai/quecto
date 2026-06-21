@done @runtime-reload
Feature: RuntimeReload — change-detection gate (Phase 2 / ADR-0002)
  As a recursive agent kernel
  I want file-backed sources to be re-read on change and rebuilt into live state
  So that an agent can extend itself and use the new state next turn without restart

  # --- ReloadSource: the mtime/hash state machine (AC6) ---

  Scenario: An unseeded source reports changed on first probe after seeding is not done
    Given a reload source for a file containing "v1"
    When I probe the source without seeding
    Then the source should report changed

  Scenario: A seeded source with no file change reports unchanged with no read
    Given a reload source for a file containing "v1"
    And the source fingerprint is seeded from the file
    When I probe the source
    Then the source should report unchanged-no-read

  Scenario: A content change is detected after mtime moves and hash differs
    Given a reload source for a file containing "v1"
    And the source fingerprint is seeded from the file
    When the file content is rewritten to "v2"
    And I probe the source
    Then the source should report changed

  # AC6b — touch-only (the bug the review caught): mtime cache MUST advance
  Scenario: A touched-but-unchanged file reports unchanged and advances the mtime cache
    Given a reload source for a file containing "v1"
    And the source fingerprint is seeded from the file
    When the file is touched with identical content
    And I probe the source
    Then the source should report unchanged
    And the source mtime cache should be advanced to the touched mtime

  Scenario: After a touch-only probe, a subsequent probe with no change performs no read
    Given a reload source for a file containing "v1"
    And the source fingerprint is seeded from the file
    And the file is touched with identical content
    And the source is probed once
    When I probe the source again
    Then the source should report unchanged-no-read

  # AC7 — missing/unreadable keeps last-good and does not crash
  Scenario: A missing file reports missing-or-unreadable and keeps the cache
    Given a reload source for a file containing "v1"
    And the source fingerprint is seeded from the file
    When the file is deleted
    And I probe the source
    Then the source should report missing-or-unreadable
    And the source cache should be unchanged

  # --- RuntimeReload: the gate + fail-safe last-good ---

  Scenario: Poll with no source changed returns unchanged and does not call rebuild
    Given a runtime reload gate watching a file containing "v1"
    And the gate is seeded with last-good "provider-v1"
    When I poll the gate with a rebuild closure
    Then the poll result should be unchanged
    And the rebuild closure should not be called

  Scenario: Poll after a content change rebuilds and returns the new value
    Given a runtime reload gate watching a file containing "v1"
    And the gate is seeded with last-good "provider-v1"
    When the file content is rewritten to "v2"
    And I poll the gate with a rebuild closure returning "provider-v2"
    Then the poll result should be reloaded with "provider-v2"
    And the gate last-good should be "provider-v2"

  Scenario: A failing rebuild keeps last-good and returns unchanged (AC7 fail-safe)
    Given a runtime reload gate watching a file containing "v1"
    And the gate is seeded with last-good "provider-v1"
    When the file content is rewritten to "broken"
    And I poll the gate with a failing rebuild closure
    Then the poll result should be unchanged
    And the gate last-good should be "provider-v1"

  # The observed fingerprint advances even on rebuild failure, so a malformed
  # file is not re-parsed every turn until it changes again.
  Scenario: After a failing rebuild, a subsequent poll with no file change does not retry
    Given a runtime reload gate watching a file containing "v1"
    And the gate is seeded with last-good "provider-v1"
    And the file content is rewritten to "broken"
    And the gate is polled with a failing rebuild closure
    When I poll the gate again with a rebuild closure
    Then the poll result should be unchanged
    And the rebuild closure should not be called

  Scenario: A malformed file that is later fixed reloads successfully (recovery)
    Given a runtime reload gate watching a file containing "v1"
    And the gate is seeded with last-good "provider-v1"
    And the file content is rewritten to "broken"
    And the gate is polled with a failing rebuild closure
    When the file content is rewritten to "v2"
    And I poll the gate with a rebuild closure returning "provider-v2"
    Then the poll result should be reloaded with "provider-v2"
    And the gate last-good should be "provider-v2"

  # Multiple sources: any change triggers rebuild
  Scenario: A gate watching two sources rebuilds when either changes
    Given a runtime reload gate watching files "a.json" containing "a1" and "b.json" containing "b1"
    And the gate is seeded with last-good "provider-v1"
    When the file "b.json" content is rewritten to "b2"
    And I poll the gate with a rebuild closure returning "provider-v2"
    Then the poll result should be reloaded with "provider-v2"

  # --- Forced poll (explicit reload trigger, §3.5) ---

  Scenario: A forced poll rebuilds even when no source changed
    Given a runtime reload gate watching a file containing "v1"
    And the gate is seeded with last-good "provider-v1"
    When I force-poll the gate with a rebuild closure returning "provider-v2"
    Then the poll result should be reloaded with "provider-v2"
    And the gate last-good should be "provider-v2"

  Scenario: A forced poll on a failing rebuild keeps last-good and returns unchanged
    Given a runtime reload gate watching a file containing "v1"
    And the gate is seeded with last-good "provider-v1"
    When I force-poll the gate with a failing rebuild closure
    Then the poll result should be unchanged
    And the gate last-good should be "provider-v1"
