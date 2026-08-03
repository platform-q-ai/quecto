@done
Feature: Stable tool identifiers
  Tool policy surfaces need provider-qualified stable identifiers while legacy name-backed policies continue to load safely.

  Scenario: Bundled native tools expose provider-qualified stable identifiers
    Given a bundled native tool named "bash" from provider "quecto:official-tools"
    When the tool catalogue is requested
    Then the catalogue entry for "bash" should have stable id "tool.v1:bundled-native:21:quecto:official-tools:bash"

  Scenario: Legacy name-backed policy identifiers resolve safely
    Given a bundled native tool named "bash" from provider "quecto:official-tools"
    When policy disables legacy tool id "bash"
    Then the catalogue entry for "bash" should be disabled

  Scenario: Renamed tool aliases resolve to the canonical tool
    Given a bundled native tool named "read" from provider "quecto:official-tools" with alias "view"
    When policy disables legacy tool id "view"
    Then the catalogue entry for "read" should be disabled

  Scenario: Duplicate stable identifiers are rejected
    Given two providers register tools with the same stable id
    When the second tool is registered
    Then registration should be rejected

  Scenario: Provider namespace collisions do not alias tools
    Given UDS tools named "weather" from providers "uds:client-a" and "uds:client-b"
    When policy disables stable tool id "tool.v1:uds:12:uds:client-a:weather"
    Then only provider "uds:client-a" tool "weather" should be disabled

  Scenario: Unknown stable identifiers remain safe
    Given a bundled native tool named "bash" from provider "quecto:official-tools"
    When policy disables stable tool id "tool.v1:bundled-native:21:quecto:official-tools:missing"
    Then the unknown policy id should be reported
