@tui
Feature: Single-connection request ownership
  As a TUI user
  I want one connection to own all pending request state
  So that only the exact response for a pending request can resolve it

  @done @issue-2044
  Scenario: A solicited transcript fetch receives a distinct correlation id
    Given a fresh headless TUI harness
    When a resume response arrives on the TUI connection
    Then the solicited transcript fetch should have a correlation id distinct from the resume response

  @done @issue-2044
  Scenario: A non-matching response id does not resolve the pending transcript fetch
    Given a fresh headless TUI harness
    And a resume response arrives on the TUI connection
    When a transcript response arrives bearing a non-matching id
    Then the pending transcript request id should remain unchanged
