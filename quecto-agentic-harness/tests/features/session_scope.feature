@issue-2009 @done
Feature: Folder-aware saved sessions through the production runtime
  Background:
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "Saved reply"

  Scenario: Resume defaults to the execution folder and explicitly discovers global sessions
    Given saved production sessions in two different folders
    When the operator opens resume through the production socket and TUI
    Then only the local saved conversation is displayed
    When the operator selects All Folders in the resume picker
    Then both saved conversations are displayed with their execution folders

  Scenario: Exact global lookup cannot reuse a foreign conversation
    Given saved production sessions in two different folders
    When the operator opens resume with the active local conversation
    And the operator requests the foreign session by exact key
    Then the runtime refuses replacement as a different-execution-directory scope error
    And the selected conversation identity history and ownership are preserved

  Scenario: Git discovery groups linked worktrees but keeps nested repositories separate
    Given saved production sessions across real Git workspaces
    When the operator opens resume through the production socket and TUI
    Then the picker groups root subfolder and linked worktree but excludes nested sessions

  Scenario: Corrupt authoritative home stays globally visible but cannot restore
    Given saved production sessions in two different folders
    And the foreign saved home metadata is corrupt
    When the operator opens resume with the active local conversation
    And the operator selects All Folders in the resume picker
    Then both saved conversations remain discoverable without repairing the corrupt home
    When the operator requests the foreign session by exact key
    Then the runtime refuses replacement as an unavailable-home scope error
    And the selected conversation identity history and ownership are preserved

  Scenario: An ephemeral runtime leaves no durable home record
    When the operator opens resume in an ephemeral production runtime
    Then the ephemeral runtime has published no transcript or home authority

  Scenario: Legacy sessions remain global and exact-key startup refuses association
    Given saved production sessions in two different folders
    And the foreign saved home metadata is absent
    When the operator opens resume through the production socket and TUI
    And the operator selects All Folders in the resume picker
    Then the foreign conversation is globally visible as unassociated
    When a fresh runtime attempts to start with the foreign session
    Then startup refuses without creating home metadata or changing the transcript

  Scenario: A corrupt catalogue rebuilds without hiding saved conversations
    Given saved production sessions in two different folders
    And the derived home catalogue is corrupt
    When the operator opens resume through the production socket and TUI
    Then only the local saved conversation is displayed
    And the discovery response reports catalogue recovery

  Scenario: Mouse scope selection and Escape do not resume history
    Given saved production sessions in two different folders
    When the operator opens resume through the production socket and TUI
    And the operator clicks All Folders in the resume picker
    Then both saved conversations are displayed with their execution folders
    When the operator cancels the resume picker
    Then no history replacement command is sent

  Scenario: Nested repository invocation uses the nearest repository
    Given saved production sessions across real Git workspaces
    And the execution directory is the nested repository
    When the operator opens resume through the production socket and TUI
    Then exactly the nested conversation is discoverable locally

  Scenario: A symlink execution alias does not duplicate the local session
    Given saved production sessions in two different folders
    And the execution directory is a symlink to the local folder
    When the operator opens resume through the production socket and TUI
    Then only the local saved conversation is displayed
    And exactly one eligible local identity is returned
