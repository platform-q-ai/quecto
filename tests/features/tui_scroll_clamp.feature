@tui @pending
Feature: TUI scroll up reaches full conversation history
  Issue #500: PageUp scroll is clamped too early. Long agent output
  cannot be fully read because scroll_offset is limited to
  entries.len() * 5 instead of actual rendered line count.

  Scenario: Scroll up reaches the top of a long conversation
    Given a chat with a 200-line assistant response
    When the user scrolls up enough times
    Then the first line of the conversation should be visible

  Scenario: Scroll offset clamps to actual rendered lines
    Given a chat with 100 rendered lines
    When the user scrolls up 200 lines
    Then the scroll offset should not exceed the rendered line count

  Scenario: Scroll down from top works
    Given the chat is scrolled to the top
    When the user scrolls down 10 lines
    Then newer content should become visible

  Scenario: New content auto-scrolls to bottom
    Given the chat is scrolled up
    When a new message arrives
    Then the scroll should reset to the bottom
