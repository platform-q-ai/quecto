@issue-2011 @issue-2045 @done
Feature: A saved session that belongs elsewhere is refused with a plain notice
  A saved session whose folder does not admit a restore here is refused —
  typed, under the asker's own id — and the client is told where it lives and
  how to open it there. Nothing is offered and nothing is ever substituted:
  no restore, no claim, no change to the conversation on screen.

  Background:
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "Saved reply"

  Scenario: An exact key from an unrelated folder resolves globally and is refused as belonging elsewhere
    Given saved production sessions in two different folders
    When the operator opens resume with the active local conversation
    And the operator requests the foreign session by exact key
    Then the runtime refuses the resume as "cross_folder" with code "belongs_elsewhere"
    And the refusal carries the command that opens quecto in the foreign folder and the resume step
    And the TUI shows the notice titled "This session belongs to another folder"
    And the selected conversation identity history and ownership are preserved

  Scenario: An exact miss never falls back to a prefix or fuzzy match
    Given saved production sessions in two different folders
    When the operator opens resume with the active local conversation
    And the operator requests the session "cli:forei" by exact key
    Then the runtime refuses the resume with code "not_found"
    And the active conversation and every claim are unchanged

  Scenario: The same execution directory restores through the socket and TUI
    Given saved production sessions in two different folders
    When the operator opens resume through the production socket and TUI
    And the operator requests the session "cli:local" by exact key
    Then the runtime answers the resume as restored to "cli:local"
    And the TUI reports the resumed session

  Scenario: A corrupt discovery index cannot block exact-key resolution
    Given saved production sessions in two different folders
    And the derived home catalogue is corrupt
    When the operator opens resume through the production socket and TUI
    And the operator requests the session "cli:local" by exact key
    Then the runtime answers the resume as restored to "cli:local"

  Scenario: A picker selection restores with the home version its row was listed at
    Given saved production sessions in two different folders
    When the operator opens resume through the production socket and TUI
    And the operator selects the listed local session
    Then the resume request carries the listed home version
    And the runtime answers the resume as restored to "cli:local"
    And the TUI reports the resumed session

  Scenario: Another row's listed version authorizes nothing for this session
    Given saved production sessions in two different folders
    When the operator opens resume through the production socket and TUI
    And the operator selects All Folders in the resume picker
    And a socket client requests the session "cli:local" with the listed version of "cli:foreign"
    Then the runtime refuses the resume with code "stale_home_version"
    And the active conversation and every claim are unchanged

  Scenario: A selection whose home changed after it was listed is refused as stale
    Given saved production sessions in two different folders
    When the operator opens resume through the production socket and TUI
    And the local session home is rewritten after the listing
    And the operator selects the listed local session
    Then the resume request carries the listed home version
    And the runtime refuses the resume with code "stale_home_version"
    And the active conversation and every claim are unchanged

  Scenario Outline: A home that cannot be observed is refused with its own kind
    Given saved production sessions in two different folders
    And the foreign session home is <state>
    When the operator opens resume with the active local conversation
    And the operator requests the session "cli:foreign" by exact key
    Then the runtime refuses the resume as "<kind>" with code "<kind>"
    And the active conversation and every claim are unchanged

    Examples:
      | state                   | kind         |
      | moved away              | home_missing |
      | permission-inaccessible | home_missing |
      | unreadable metadata     | home_unknown |

  Scenario: A session with no folder recorded is refused and gains none
    Given saved production sessions in two different folders
    And the foreign saved home metadata is absent
    When the operator opens resume with the active local conversation
    And the operator requests the session "cli:foreign" by exact key
    Then the runtime refuses the resume as "legacy_unscoped" with code "no_home_recorded"
    And no home metadata was created for the foreign session
    And the active conversation and every claim are unchanged

  Scenario: Closing the notice changes nothing and sends nothing
    Given saved production sessions in two different folders
    When the operator opens resume with the active local conversation
    And the operator requests the foreign session by exact key
    And the operator closes the notice
    Then the notice is closed and no command was sent
    And the active conversation and every claim are unchanged

  Scenario Outline: A request that still names an action is refused under its own id and restores nothing
    Given saved production sessions in two different folders
    When the operator opens resume with the active local conversation
    And a socket client requests the foreign session with action "<action>"
    Then the runtime refuses the resume with code "legacy_action_unsupported"
    And the active conversation and every claim are unchanged

    Examples:
      | action         |
      | open_original  |
      | fork_current   |
      | cancel         |
      | restore_anyway |

  Scenario: Startup repeats the guard for a session saved in another folder
    Given saved production sessions in two different folders
    When a fresh runtime attempts to start with the foreign session
    Then startup refuses naming the other execution directory
    And the foreign transcript and home metadata are unchanged

  Scenario: Invisible reordering characters in a recorded folder never reach a client
    Given saved production sessions in two different folders
    And the foreign session home names a directory with bidi and zero-width characters
    When the operator opens resume with the active local conversation
    And the operator requests the session "cli:foreign" by exact key
    Then the runtime refuses the resume as "home_missing" with code "home_missing"
    And the refusal carries no command
    And neither the answer nor the TUI frame carries a bidi or zero-width character

  Scenario: Hostile home metadata is rendered safely in the notice
    Given saved production sessions in two different folders
    And the foreign session home names a directory with terminal control characters
    When the operator opens resume with the active local conversation
    And the operator requests the session "cli:foreign" by exact key
    Then the runtime refuses the resume as "home_unknown" with code "home_unknown"
    And the refusal carries no command
    And neither the answer nor the TUI frame carries a raw control character
