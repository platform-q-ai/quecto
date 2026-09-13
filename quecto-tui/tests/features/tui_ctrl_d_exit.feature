@tui @done
Feature: TUI Ctrl+D exits the app unconditionally
  As a TUI user
  I want Ctrl+D to always exit the app
  So that I can reliably quit regardless of UI state

  Scenario: Ctrl+D exits with no overlay active
    Given the TUI is running with no overlays
    When the user presses Ctrl+D
    Then the app should set should_exit to true
    And the main loop should break

  Scenario: Ctrl+D exits even with overlay active
    Given a confirm overlay is active
    When the user presses Ctrl+D
    Then the app should exit
    And the overlay should not consume the key

  Scenario: Ctrl+D requests ordinary exit during response
    Given the agent is streaming a response
    When the user presses Ctrl+D
    Then the active agent should continue without a Ctrl-D abort
    And then the app should exit

  Scenario: Ctrl+D exits without editing a non-empty draft
    Given the editor contains "draft text"
    When the user presses Ctrl+D
    Then the app should exit
    And the editor should still contain "draft text"

  Scenario: Ctrl+D exits with autocomplete active
    Given the autocomplete dropdown is showing
    When the user presses Ctrl+D
    Then the app should exit

  # #1956: ordinary exit signals the harness leader only, waits within the
  # budget derived from the harness fleet teardown, SIGKILLs only that pid
  # past it, and reports (never signals) strays afterwards.

  @leader-exit
  Scenario: Ordinary exit lets the owned harness settle its own child before exiting
    Given the TUI owns a stand-in harness that settles its own child on SIGTERM
    When the TUI finalizes ordinary exit
    Then the harness child should be gone before the harness exited
    And the harness should have exited after SIGTERM without SIGKILL
    And the harness child should have received no signal from the TUI
    And the exit report should mention no SIGKILL and no stray

  @leader-exit
  Scenario: A slow-settling harness shows the settling notice and exits within the budget
    Given the TUI owns a stand-in harness that exits 2 seconds after SIGTERM
    When the TUI finalizes ordinary exit
    Then the settling notice should have been shown with elapsed seconds
    And the settling notice should be dismissed
    And the harness should have exited after SIGTERM without SIGKILL
    And the exit report should mention no SIGKILL and no stray

  @leader-exit
  Scenario: A harness ignoring SIGTERM is SIGKILLed only after the budget
    Given the TUI owns a stand-in harness that ignores SIGTERM
    And the leader exit budget is 300 milliseconds
    When the TUI finalizes ordinary exit
    Then the harness should have been SIGKILLed after the budget
    And the exit report should name the SIGKILLed harness pid

  @leader-exit
  Scenario: The post-exit canary names a stray process without signalling it
    Given the TUI owns a stand-in harness whose child outlives it
    When the TUI finalizes ordinary exit
    Then the exit report should name the stray pid as not signalled
    And the stray process should still be alive and unsignalled

  @leader-exit
  Scenario: Detach-on-exit leaves the owned harness running
    Given the TUI owns a stand-in harness that settles its own child on SIGTERM
    And the exit policy is detach-on-exit
    When the TUI finalizes ordinary exit
    Then the harness should still be running
    And the harness child should still be running

  @leader-exit
  Scenario: Tab close and /new terminate through the same leader-only helper
    Given the TUI owns a stand-in harness that settles its own child on SIGTERM
    When the owned harness is terminated through the watcher with a 10 second budget
    Then the watcher should report the harness exited after SIGTERM
    And the harness child should be gone before the harness exited
    And the harness child should have received no signal from the TUI
