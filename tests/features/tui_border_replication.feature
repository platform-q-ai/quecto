@tui
Feature: TUI editor borders render without duplication
  As a TUI user
  I want the input area borders to render cleanly during streaming
  So that the UI doesn't show duplicated or garbled borders

  Scenario: Editor borders stable during agent response
    Given the agent is streaming tokens
    And the spinner is active above the editor
    When the screen re-renders on each token
    Then the editor should show exactly one top border and one bottom border

  Scenario: Alternate screen buffer prevents scrollback interference
    Given the TUI uses the alternate screen buffer
    When content is rendered via cursor home
    Then scrollback does not cause position errors
