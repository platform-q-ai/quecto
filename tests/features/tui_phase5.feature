@tui @pending
Feature: TUI Phase 5 — Overlay System and Settings UI
  Modal overlays, settings, bash mode, confirm dialogs, notifications.

  Scenario: Overlay composites on top of base content
    Given base content lines "line1", "line2", "line3", "line4"
    And an overlay with content "OVERLAY" at row 1 col 5
    When the overlay composites at width 40 height 4
    Then line 1 should contain "OVERLAY" starting at column 5
    And line 0 should still contain "line1"

  Scenario: Overlay centers by default
    Given base content of 10 lines
    And a centered overlay with content "dialog" width 20
    When the overlay composites at width 80 height 10
    Then the overlay should be approximately centered

  Scenario: Overlay captures keyboard focus
    Given an active overlay
    When the user presses a key
    Then the overlay should receive the input
    And the base editor should not receive the input

  Scenario: Overlay stack — topmost gets input
    Given two stacked overlays A and B
    When the user presses a key
    Then overlay B (topmost) should receive the input

  Scenario: Hiding an overlay restores focus
    Given an overlay with captured focus
    When the overlay is hidden
    Then focus should return to the previous component

  Scenario: Confirm dialog blocks on Yes/No
    Given a confirm dialog with message "Clear history?"
    When the user presses Enter (Yes)
    Then the result should be confirmed

  Scenario: Confirm dialog cancels on Escape
    Given a confirm dialog with message "Clear history?"
    When the user presses Escape
    Then the result should be cancelled

  Scenario: Notification renders and auto-dismisses
    Given a notification "Saved!" with type "success"
    When the notification renders at width 40
    Then the rendered output should contain "Saved!"
