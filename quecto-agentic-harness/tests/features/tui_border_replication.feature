@tui @done
Feature: TUI editor borders render without duplication
  As a TUI user
  I want the input area borders to render cleanly during streaming
  So that the UI doesn't show duplicated or garbled borders

  Scenario: Editor borders stable during agent response
    Given the agent is streaming tokens
    And the spinner is active above the editor
    When the screen re-renders on each token
    Then the editor should show exactly one top border and one bottom border

  Scenario: Pasted multi-line content keeps a single clean editor frame
    Given an editor component with text ""
    When the user pastes "alpha\r\nbeta\r\ngamma"
    And the editor renders at width 40 three times
    Then each render should show exactly one top border and one bottom border
    And the rendered output should contain "alpha"
    And the rendered output should contain "beta"
    And the rendered output should contain "gamma"

  Scenario: Alternate screen buffer prevents scrollback interference
    Given the TUI uses the alternate screen buffer
    When content is rendered via cursor home
    Then scrollback does not cause position errors
