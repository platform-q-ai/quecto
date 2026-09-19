@issue-2010 @done
Feature: Global session metadata search through the production runtime
  The /resume picker's search box asks the production harness, over its real
  socket, for sessions whose title, exact key, repository label or path match.

  Background:
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "Saved reply"

  Scenario: Title, key, repository and path each find exactly their session in All Folders
    Given saved production sessions with distinct title key repository and path
    When the operator opens resume through the production socket and TUI
    And the operator selects All Folders in the resume picker
    And the operator searches the resume picker for "zebra-title"
    Then the searched sessions displayed are exactly "ZEBRA-TITLE"
    When the operator searches the resume picker for "cli:bykey"
    Then the searched sessions displayed are exactly "KEYED-CONVERSATION"
    When the operator searches the resume picker for "walrus-repo"
    Then the searched sessions displayed are exactly "REPO-CONVERSATION"
    When the operator searches the resume picker for "deep/heron"
    Then the searched sessions displayed are exactly "PATH-CONVERSATION"
    And every search was answered by the production search command in scope "global"

  Scenario: Unscoped sessions are searchable and labelled and Local Folder restores local-only results
    Given saved production sessions with distinct title key repository and path
    When the operator opens resume through the production socket and TUI
    And the operator selects All Folders in the resume picker
    And the operator searches the resume picker for "otter"
    Then the searched sessions displayed are exactly "LOCAL-CONVERSATION otter, LEGACY otter"
    And the searched row "LEGACY otter" is labelled unscoped with its stable key "cli:legacy"
    And the searched row "LOCAL-CONVERSATION otter" shows its execution folder and its stable key "cli:local"
    When the operator switches the resume picker back to Local Folder
    Then the searched sessions displayed are exactly "LOCAL-CONVERSATION otter"
    And the last search was answered in scope "local"

  Scenario: Transcript content is never matched
    Given saved production sessions with distinct title key repository and path
    When the operator opens resume through the production socket and TUI
    And the operator selects All Folders in the resume picker
    And the operator searches the resume picker for "Saved reply"
    Then no searched session is displayed
    And the search answer reports every saved session searched and none matched

  Scenario: An answer overtaken by more typing is discarded
    Given saved production sessions with distinct title key repository and path
    When the operator opens resume through the production socket and TUI
    And the operator selects All Folders in the resume picker
    And the operator types "z" then "ebra" before the first answer arrives
    Then the first answer is discarded and the listing stays on screen
    And the answer to "zebra" replaces the listing with exactly "ZEBRA-TITLE"

  Scenario: Hostile queries are literal bounded and deterministic
    Given saved production sessions with distinct title key repository and path
    When the operator opens resume through the production socket and TUI
    Then a production search for each hostile query is answered safely the same way twice
    And a production search for a query of 100000 characters is refused without searching

  Scenario: A session saved in a non-UTF-8 folder is found and shown safely
    Given a production session saved in a folder whose name is not UTF-8
    When the operator opens resume through the production socket and TUI
    And the operator selects All Folders in the resume picker
    And the operator searches the resume picker for "caf"
    Then the searched sessions displayed are exactly "ODD-FOLDER-CONVERSATION"
    And the search answer is valid UTF-8 with a replacement character in the execution path

  Scenario: A session deleted between the search and the selection activates nothing
    Given saved production sessions with distinct title key repository and path
    When the operator opens resume with the active local conversation
    And the operator selects All Folders in the resume picker
    And the operator searches the resume picker for "zebra-title"
    And the searched session is deleted before the operator selects it
    Then the selection is refused as not found and the active conversation is unchanged

  Scenario: A session re-homed between the search and the selection is refused as stale
    Given saved production sessions with distinct title key repository and path
    When the operator opens resume with the active local conversation
    And the operator selects All Folders in the resume picker
    And the operator searches the resume picker for "zebra-title"
    And the searched session is re-homed before the operator selects it
    Then the selection is refused as a stale home version and the active conversation is unchanged

  Scenario: Escape after a search changes nothing
    Given saved production sessions with distinct title key repository and path
    When the operator opens resume with the active local conversation
    And the operator selects All Folders in the resume picker
    And the operator searches the resume picker for "zebra-title"
    And the operator cancels the resume picker
    Then no resume was requested and no saved session or home changed

  Scenario: A corrupt index and a corrupt record are diagnosed without hiding a match
    Given saved production sessions with distinct title key repository and path
    And the derived home catalogue is corrupt
    And a corrupt session record sits beside the saved sessions
    When the operator opens resume with the active local conversation
    And the operator selects All Folders in the resume picker
    And the operator searches the resume picker for "zebra-title"
    Then the searched sessions displayed are exactly "ZEBRA-TITLE"
    And the search answers name the corrupt record and a later search reports no recovery
    And exact-key resume of "cli:bytitle" is still answered with a decision
