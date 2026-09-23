@tui @efficiency
Feature: Chat render cache remains bounded in long sessions (#981)
  As a TUI user in a long-running workflow
  I want chat history to remain available without retaining rendered copies of the whole session
  So that memory use stays proportional to the visible conversation window

  @done @issue-2099
  Scenario: Historical scrollback is rendered on demand
    Given the chat transcript is much longer than the visible viewport
    And the latest conversation window has been rendered
    When the user scrolls back to older history
    Then the older conversation window is rendered correctly
    And the scroll position still identifies the requested history
