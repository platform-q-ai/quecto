@tui @done
Feature: Waiting for inference admission is visible without becoming a lifecycle state
  A queued attempt shows up as a waiting label on the footer and the working
  spinner, a descendant's forwarded wait shows on its panel row, and a
  granted or finished run drops the label again; the session is never shown
  idle or stalled because of admission.

  Scenario: The master's queued attempt is shown as waiting and cleared on grant
    Given a fresh TUI harness for admission scenarios
    When the agent starts a run
    And the agent reports its admission view as waiting for 12 seconds in group "anthropic"
    Then the master footer shows "12s"
    And the working spinner says "⏳ 12s (Esc to interrupt)"
    When the agent reports its admission view as admitted
    Then the master footer shows no admission label
    And the working spinner says "Working... (Esc to interrupt)"

  Scenario: A cooldown survives the end of the run but a wait does not
    Given a fresh TUI harness for admission scenarios
    When the agent starts a run
    And the agent reports a 30 second cooldown for group "anthropic"
    And the agent ends the run
    Then the master footer shows "anthropic cooldown 30s"
    When the agent starts a run
    And the agent reports its admission view as waiting for 3 seconds in group "anthropic"
    And the agent ends the run
    Then the master footer shows no admission label

  Scenario: A descendant's forwarded wait is painted on its panel row only
    Given a fresh TUI harness for admission scenarios
    And a running sub-agent "reviewer" is on the roster
    When the parent forwards "reviewer" waiting for admission for 4 seconds
    Then the sub-agent panel row for "reviewer" shows "4s"
    And the master footer shows no admission label
    When the parent forwards "reviewer" with nothing waiting
    Then the sub-agent panel row for "reviewer" shows no admission label

  Scenario: The slim state carries the admission view like the pushed event
    Given a fresh TUI harness for admission scenarios
    When a get_state response arrives with a waiting admission view of 7 seconds
    Then the master footer shows "7s"
