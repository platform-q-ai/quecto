@tui @done @issue-2011
Feature: /resume renders the harness's typed resume decision (#2011)
  As a TUI user resuming a session that belongs elsewhere
  I want the explicit choices the harness offers, with unavailable ones explained
  So that nothing is restored, linked or substituted without my explicit action

  Background:
    Given a fresh TUI app harness

  Scenario: A typed exact key sends the key alone
    When I submit the master prompt "/resume cli:foreign"
    Then one resume request is sent for "cli:foreign" with no action and no version

  Scenario Outline: A decision answer opens the dialog with the offered actions in order
    When I submit the master prompt "/resume cli:foreign"
    And the harness answers the resume with a "<kind>" decision offering "<actions>" where "cancel" is available
    Then the decision dialog is titled "<title>"
    And the decision dialog lists "<labels>" in order
    And every action except Cancel is marked unavailable

    Examples:
      | kind            | actions                           | title                                                                    | labels                                                             |
      | cross_folder    | open_original,fork_current,cancel | This session belongs to another folder                                  | Open original folder,Copy into this folder as a new session,Cancel |
      | home_missing    | locate,fork_current,cancel        | This session's folder can't be opened (missing, moved or no permission) | Locate folder,Copy into this folder as a new session,Cancel        |
      | home_changed    | locate,fork_current,cancel        | This folder is no longer the same project as when the session was saved | Locate folder,Copy into this folder as a new session,Cancel        |
      | home_unknown    | locate,fork_current,cancel        | quecto can't read where this session was saved                          | Locate folder,Copy into this folder as a new session,Cancel        |
      | legacy_unscoped | associate,cancel                  | This session was saved before quecto tracked folders                    | Attach to a folder,Cancel                                          |

  Scenario Outline: The dialog is legible on an ordinary and on a small terminal
    Given a fresh TUI app harness on a <columns> by <rows> terminal
    When I submit the master prompt "/resume cli:foreign"
    And the harness answers the resume with a "<kind>" decision for a session saved in a long folder path
    Then the decision dialog box is whole on the <columns> by <rows> terminal
    And the decision dialog shows both ends of the recorded folder
    And every unavailable decision row carries its mark and Cancel carries none
    And the decision dialog title and every action label read whole
    And the decision dialog shows the whole reason of the unavailable action under the cursor

    Examples:
      | columns | rows | kind            |
      | 80      | 24   | cross_folder    |
      | 80      | 24   | legacy_unscoped |
      | 40      | 20   | cross_folder    |
      | 40      | 20   | legacy_unscoped |

  Scenario: A folder that cannot be read says why under the folder
    When I submit the master prompt "/resume cli:foreign"
    And the harness answers the resume with a "home_missing" decision whose detail is "Permission denied (os error 13)"
    Then the decision dialog is titled "can't be opened (missing, moved or no permission)"
    And the decision dialog shows "Permission denied (os error 13)"

  Scenario: The dialog names the session by the title picked in the list
    When I submit the master prompt "/resume"
    And the harness lists the session "cli:foreign" titled "hello from A" saved in another folder
    Then the resume list explains the row with "Saved in another folder"
    When I pick the listed session
    And the harness answers the resume with a "cross_folder" decision offering "open_original,fork_current,cancel" where "cancel" is available
    Then the decision dialog shows "hello from A"
    And the decision dialog shows "cli:foreign"

  Scenario: Ctrl-C in the dialog closes it and sends nothing
    When I submit the master prompt "/resume cli:foreign"
    And the harness answers the resume with a "cross_folder" decision offering "open_original,fork_current,cancel" where "fork_current,cancel" is available
    And I press Ctrl-C in the decision dialog
    Then the decision dialog is closed
    And the decision dialog sent no command

  Scenario Outline: A success that is not a restore changes nothing in the TUI
    Given the TUI shows the session "cli:local"
    When I submit the master prompt "/resume cli:foreign"
    And the harness answers the resume as a success with outcome "<outcome>" for "cli:foreign"
    Then the TUI still shows the session "cli:local"
    And the TUI never reports a resumed session
    And the TUI explains "Nothing changed: update quecto-tui"
    And the answer made the TUI send nothing

    Examples:
      | outcome          |
      | opened_elsewhere |
      | forked           |
      | decision         |
      | refused          |

  Scenario: Another tab's refusal is not reported here
    When another client's resume is refused with code "not_found"
    Then the decision dialog is closed
    And the TUI reports nothing

  Scenario: An action the harness cannot parse settles the resume instead of waiting for ever
    When I submit the master prompt "/resume cli:foreign"
    And the harness answers the resume with a "cross_folder" decision offering "open_original,fork_current,cancel" where "fork_current,cancel" is available
    And I choose decision row 2
    And the harness rejects the resume line with an uncorrelated parse error
    Then the TUI explains "Resume failed"
    And no resume request is left in flight

  Scenario: Another client's unparsed action does not settle this tab's plain restore
    When I submit the master prompt "/resume cli:foreign"
    And the harness rejects the resume line with an uncorrelated parse error
    Then the TUI reports nothing
    And the resume request is still in flight
    When the harness answers the resume with a "cross_folder" decision offering "open_original,fork_current,cancel" where "cancel" is available
    Then the decision dialog is titled "This session belongs to another folder"

  Scenario: Escape closes the dialog and sends nothing
    When I submit the master prompt "/resume cli:foreign"
    And the harness answers the resume with a "cross_folder" decision offering "open_original,fork_current,cancel" where "cancel" is available
    And I press Escape in the decision dialog
    Then the decision dialog is closed
    And the decision dialog sent no command

  Scenario: Choosing Cancel closes the dialog and sends nothing
    When I submit the master prompt "/resume cli:foreign"
    And the harness answers the resume with a "legacy_unscoped" decision offering "associate,cancel" where "cancel" is available
    And I choose decision row 2
    Then the decision dialog is closed
    And the decision dialog sent no command

  Scenario: An unavailable action is pointed at its reason, kept on screen and never sent
    When I submit the master prompt "/resume cli:foreign"
    And the harness answers the resume with a "cross_folder" decision offering "open_original,fork_current,cancel" where "cancel" is available
    And I choose decision row 1
    Then the TUI explains "Not available yet — see the reason below"
    And the decision dialog shows "open_original is not delivered yet"
    And the decision dialog is titled "belongs to another folder"
    And the decision dialog sent no command

  Scenario: An available action sends stable identity, action and the decided version
    When I submit the master prompt "/resume cli:foreign"
    And the harness answers the resume with a "cross_folder" decision offering "open_original,fork_current,cancel" where "fork_current,cancel" is available
    And I choose decision row 2
    Then the decision dialog is closed
    And one resume request is sent for "cli:foreign" with action "fork_current" and the decided version

  Scenario Outline: A typed refusal is a toast, never a dialog
    When I submit the master prompt "/resume cli:foreign"
    And the harness refuses the resume with code "<code>"
    Then the decision dialog is closed
    And the TUI explains "<toast>"

    Examples:
      | code               | toast                                            |
      | claim_refused      | Resume failed                                    |
      | stale_home_version | List out of date — reopen /resume and pick again |

  Scenario: Another tab's decision opens nothing here
    When another client's resume is answered with a decision
    Then the decision dialog is closed

  Scenario: A decision with an action this TUI does not know is a plain refusal
    When I submit the master prompt "/resume cli:foreign"
    And the harness answers the resume with a "cross_folder" decision offering "restore_anyway,cancel" where "restore_anyway,cancel" is available
    Then the decision dialog is closed
    And the TUI explains "Resume failed"

  Scenario: Invisible reordering characters in a decision never reach the terminal
    When I submit the master prompt "/resume cli:foreign"
    And the harness answers the resume with a decision carrying bidi and zero-width characters
    Then the decision dialog is titled "belongs to another folder"
    And the rendered frame carries no bidi or zero-width character

  Scenario: Hostile decision metadata never reaches the terminal raw
    When I submit the master prompt "/resume cli:foreign"
    And the harness answers the resume with a decision carrying terminal control characters
    Then the decision dialog is titled "quecto can't read where this session was saved"
    And the rendered frame carries no raw control sequence from the decision
