@tui @efficiency @done
Feature: Idle TUI avoids unnecessary periodic work (#978)
  As a TUI user on a battery-powered machine
  I want the interface to stay quiet when nothing is changing
  So that leaving it open does not waste CPU or battery

  Scenario: Quiet sessions do not perform sub-second periodic work
    Given the TUI has no visible animation
    And no notification is active
    And no subagent is active
    And no response is streaming
    When the session is left idle
    Then the TUI performs no sub-second periodic work

  Scenario: Activity spinner keeps progressing visibly
    Given the activity spinner is visible
    When the session is left idle
    Then the activity spinner progresses

  Scenario: Notifications remain serviced while visible
    Given a notification is visible
    When the session is left idle
    Then the notification remains serviced until it is no longer visible

  Scenario: Branch changes are reflected promptly
    Given the branch indicator shows the current branch
    When the repository switches to another branch
    Then the branch indicator shows the new branch within a few seconds

  Scenario: Unsupported terminals receive keyboard fallback
    Given the terminal does not confirm Kitty keyboard protocol support
    When the fallback detection deadline passes
    Then the TUI enables keyboard fallback mode
    And normal keyboard input is accepted
