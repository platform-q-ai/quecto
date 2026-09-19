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
      | kind            | actions                           | title                        | labels                                                 |
      | cross_folder    | open_original,fork_current,cancel | belongs to another folder    | Open original folder,Fork into current folder,Cancel   |
      | home_missing    | locate,fork_current,cancel        | folder is missing or moved   | Locate folder,Fork into current folder,Cancel          |
      | home_changed    | locate,fork_current,cancel        | folder changed its workspace | Locate folder,Fork into current folder,Cancel          |
      | home_unknown    | locate,fork_current,cancel        | folder record is unreadable  | Locate folder,Fork into current folder,Cancel          |
      | legacy_unscoped | associate,cancel                  | never linked to a folder     | Associate with a folder,Cancel                         |

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

  Scenario: An unavailable action is explained, kept on screen and never sent
    When I submit the master prompt "/resume cli:foreign"
    And the harness answers the resume with a "cross_folder" decision offering "open_original,fork_current,cancel" where "cancel" is available
    And I choose decision row 1
    Then the TUI explains "Open original folder is unavailable"
    And the decision dialog is titled "belongs to another folder"
    And the decision dialog sent no command

  Scenario: An available action sends stable identity, action and the decided version
    When I submit the master prompt "/resume cli:foreign"
    And the harness answers the resume with a "cross_folder" decision offering "open_original,fork_current,cancel" where "fork_current,cancel" is available
    And I choose decision row 2
    Then the decision dialog is closed
    And one resume request is sent for "cli:foreign" with action "fork_current" and the decided version

  Scenario: A typed refusal is a toast, never a dialog
    When I submit the master prompt "/resume cli:foreign"
    And the harness refuses the resume with code "stale_home_version"
    Then the decision dialog is closed
    And the TUI explains "Resume failed"

  Scenario: Another tab's decision opens nothing here
    When another client's resume is answered with a decision
    Then the decision dialog is closed

  Scenario: A decision with an action this TUI does not know is a plain refusal
    When I submit the master prompt "/resume cli:foreign"
    And the harness answers the resume with a "cross_folder" decision offering "restore_anyway,cancel" where "restore_anyway,cancel" is available
    Then the decision dialog is closed
    And the TUI explains "Resume failed"

  Scenario: Hostile decision metadata never reaches the terminal raw
    When I submit the master prompt "/resume cli:foreign"
    And the harness answers the resume with a decision carrying terminal control characters
    Then the decision dialog is titled "folder record is unreadable"
    And the rendered frame carries no raw control sequence from the decision
