@tui @done @issue-2011 @issue-2045
Feature: /resume shows a plain notice when the harness refuses a session that lives elsewhere (#2045)
  As a TUI user resuming a session saved in another folder
  I want to be told where it lives and how to open it there
  So that nothing is restored or sent on my behalf, and I know what to do next

  Background:
    Given a fresh TUI app harness

  Scenario: A typed exact key sends the key alone
    When I submit the master prompt "/resume cli:foreign"
    Then one resume request is sent for "cli:foreign" with no action and no version

  Scenario Outline: A refusal about the session's folder opens the notice in plain words
    When I submit the master prompt "/resume cli:foreign"
    And the harness refuses the resume as "<kind>" with code "<code>"
    Then the notice is titled "<title>"
    And the notice offers nothing to choose
    And no resume request is left in flight

    Examples:
      | kind            | code              | title                                          |
      | cross_folder    | belongs_elsewhere | This session belongs to another folder         |
      | home_missing    | home_missing      | This session's folder is missing or unreadable |
      | home_changed    | home_changed      | This session's folder has changed              |
      | home_unknown    | home_unknown      | This session's folder can't be read            |
      | legacy_unscoped | no_home_recorded  | No folder is recorded for this session         |

  Scenario Outline: The notice is whole on an ordinary and on a small terminal
    Given a fresh TUI app harness on a <columns> by <rows> terminal
    When I submit the master prompt "/resume cli:foreign"
    And the harness refuses the resume for a session saved in a long folder path
    Then the notice box is whole on the <columns> by <rows> terminal
    And the notice shows the whole command and the resume step
    And the notice shows both ends of the recorded folder

    Examples:
      | columns | rows |
      | 120     | 40   |
      | 80      | 24   |
      | 40      | 20   |

  Scenario: The command says how to open quecto there and never names a session flag
    When I submit the master prompt "/resume cli:foreign"
    And the harness refuses the resume as "cross_folder" with code "belongs_elsewhere"
    Then the notice shows "cd '/work/elsewhere' && quecto-tui"
    And the notice shows "/resume cli:foreign"
    And the notice never shows "quecto-tui -s"

  Scenario Outline: Only a session that lives elsewhere is told to go there
    When I submit the master prompt "/resume cli:foreign"
    And the harness refuses the resume as "<kind>" with code "<code>"
    Then the notice shows "<why>"
    And the notice never shows "quecto-tui"
    And the notice never shows "/resume cli:foreign"

    Examples:
      | kind            | code             | why                           |
      | home_missing    | home_missing     | Bring that folder back        |
      | home_changed    | home_changed     | is a different project now    |
      | home_unknown    | home_unknown     | folder record can't be read   |
      | legacy_unscoped | no_home_recorded | before quecto tracked folders |

  Scenario: A folder that cannot be read says why under the folder
    When I submit the master prompt "/resume cli:foreign"
    And the harness refuses the resume as "home_missing" with detail "Permission denied (os error 13)"
    Then the notice shows "Permission denied (os error 13)"

  Scenario: The notice names the session by the title picked in the list
    When I submit the master prompt "/resume"
    And the harness lists the session "cli:foreign" titled "Fix the flaky renderer" saved in another folder
    Then the resume list explains the row with "In another folder"
    When I pick the listed session
    And the harness refuses the resume as "cross_folder" with code "belongs_elsewhere"
    Then the notice shows "Fix the flaky renderer"

  Scenario Outline: Every key that closes the notice sends nothing
    When I submit the master prompt "/resume cli:foreign"
    And the harness refuses the resume as "cross_folder" with code "belongs_elsewhere"
    And I press <key> in the notice
    Then the notice is closed
    And the notice sent no command

    Examples:
      | key    |
      | Escape |
      | Enter  |
      | Ctrl-C |

  Scenario Outline: A success that is not a restore changes nothing in the TUI
    Given the TUI shows the session "cli:local"
    When I submit the master prompt "/resume cli:foreign"
    And the harness answers the resume as a success with outcome "<outcome>" for "cli:foreign"
    Then the TUI still shows the session "cli:local"
    And the TUI never reports a resumed session
    And the answer made the TUI send nothing

    Examples:
      | outcome          |
      | opened_elsewhere |
      | forked           |
      | decision         |
      | refused          |

  Scenario Outline: A refusal that is not about the folder is a toast, never the notice
    When I submit the master prompt "/resume cli:foreign"
    And the harness refuses the resume with code "<code>"
    Then the notice is closed
    And the TUI explains "<toast>"

    Examples:
      | code                      | toast                                            |
      | not_found                 | Resume failed                                    |
      | claim_refused             | Resume failed                                    |
      | legacy_action_unsupported | Resume failed                                    |
      | stale_home_version        | List out of date — reopen /resume and pick again |

  Scenario: Another client's refusal is not reported here and opens nothing
    When another client's resume is refused as belonging elsewhere
    Then the notice is closed
    And the TUI reports nothing

  Scenario: An older harness's decision still reads as the same notice with nothing to run
    When I submit the master prompt "/resume cli:foreign"
    And an older harness answers the resume with a "cross_folder" decision
    Then the notice is titled "This session belongs to another folder"
    And the notice shows "Open quecto in that folder and resume it there."

  Scenario: A command the terminal could not show as written is not shown at all
    When I submit the master prompt "/resume cli:foreign"
    And the harness refuses the resume with a command carrying terminal control characters
    Then the notice never shows "quecto-tui"
    And the notice shows "Open quecto in that folder, then type:"
    And the rendered frame carries no raw control sequence

  Scenario: Invisible reordering characters in a refusal never reach the terminal
    When I submit the master prompt "/resume cli:foreign"
    And the harness refuses the resume with a folder carrying bidi and zero-width characters
    Then the rendered frame carries no bidi or zero-width character
