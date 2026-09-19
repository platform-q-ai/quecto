@issue-2011 @done
Feature: Typed resume decisions through the production runtime
  A saved session whose home does not admit a restore here is answered with an
  explicit, capability-driven decision — never a silent restore, never another
  action in place of the one asked for.

  Background:
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "Saved reply"

  Scenario: An exact key from an unrelated folder resolves globally into a cross-folder decision
    Given saved production sessions in two different folders
    When the operator opens resume with the active local conversation
    And the operator requests the foreign session by exact key
    Then the runtime answers a "cross_folder" decision offering "open_original,fork_current,cancel"
    And every offered action except cancel is unavailable with a reason
    And the TUI shows the decision dialog titled "belongs to another folder"
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

  Scenario: A selection whose home changed after it was listed is refused as stale
    Given saved production sessions in two different folders
    When the operator opens resume through the production socket and TUI
    And the local session home is rewritten after the listing
    And the operator selects the listed local session
    Then the resume request carries the listed home version
    And the runtime refuses the resume with code "stale_home_version"
    And the active conversation and every claim are unchanged

  Scenario Outline: A home that cannot be observed yields an explicit decision
    Given saved production sessions in two different folders
    And the foreign session home is <state>
    When the operator opens resume with the active local conversation
    And the operator requests the session "cli:foreign" by exact key
    Then the runtime answers a "<kind>" decision offering "locate,fork_current,cancel"
    And every offered action except cancel is unavailable with a reason
    And the active conversation and every claim are unchanged

    Examples:
      | state                   | kind         |
      | moved away              | home_missing |
      | permission-inaccessible | home_missing |
      | unreadable metadata     | home_unknown |

  Scenario: A legacy session requires an explicit first association
    Given saved production sessions in two different folders
    And the foreign saved home metadata is absent
    When the operator opens resume with the active local conversation
    And the operator requests the session "cli:foreign" by exact key
    Then the runtime answers a "legacy_unscoped" decision offering "associate,cancel"
    And every offered action except cancel is unavailable with a reason
    And the decision message keeps the explicit-association guidance
    And no home metadata was created for the foreign session
    And the active conversation and every claim are unchanged

  Scenario: Cancel at the decision dialog changes nothing
    Given saved production sessions in two different folders
    When the operator opens resume with the active local conversation
    And the operator requests the foreign session by exact key
    And the operator cancels the decision dialog
    Then the decision dialog is closed and no command was sent
    And the active conversation and every claim are unchanged

  Scenario: An unavailable action chosen in the dialog is explained and never sent
    Given saved production sessions in two different folders
    When the operator opens resume with the active local conversation
    And the operator requests the foreign session by exact key
    And the operator chooses the first offered action in the decision dialog
    Then the TUI explains the action is unavailable and keeps the dialog open
    And the decision dialog sent no command

  Scenario Outline: A direct socket request for an action is never substituted
    Given saved production sessions in two different folders
    When the operator opens resume with the active local conversation
    And a socket client requests the foreign session with action "<action>"
    Then the runtime refuses the resume with code "<code>"
    And the active conversation and every claim are unchanged

    Examples:
      | action        | code               |
      | open_original | action_unavailable |
      | fork_current  | action_unavailable |
      | locate        | action_unavailable |
      | associate     | action_unavailable |

  Scenario: A direct socket cancel is acknowledged without any effect
    Given saved production sessions in two different folders
    When the operator opens resume with the active local conversation
    And a socket client requests the foreign session with action "cancel"
    Then the runtime answers the resume as cancelled
    And the active conversation and every claim are unchanged

  Scenario: An unknown action on the socket is rejected at the protocol boundary
    Given saved production sessions in two different folders
    When the operator opens resume with the active local conversation
    And a socket client requests the foreign session with action "restore_anyway"
    Then the socket rejects the malformed resume request
    And the active conversation and every claim are unchanged

  Scenario: Startup repeats the guard for a session saved in another folder
    Given saved production sessions in two different folders
    When a fresh runtime attempts to start with the foreign session
    Then startup refuses naming the other execution directory
    And the foreign transcript and home metadata are unchanged

  Scenario: Hostile home metadata is rendered safely in the decision
    Given saved production sessions in two different folders
    And the foreign session home names a directory with terminal control characters
    When the operator opens resume with the active local conversation
    And the operator requests the session "cli:foreign" by exact key
    Then the runtime answers a "home_unknown" decision offering "locate,fork_current,cancel"
    And neither the answer nor the TUI frame carries a raw control character
