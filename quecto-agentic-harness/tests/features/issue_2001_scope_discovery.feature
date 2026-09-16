@issue-2001 @scope-discovery
Feature: Discover saved-session candidates by folder and Git workspace identity
  As a user browsing saved conversations
  I want discovery to follow canonical folder and Git workspace grouping
  So that the picker includes related candidates without exposing unrelated folders

  Discovery grouping only controls which candidates are shown. It does not make a
  session immediately resumable: that separately requires the same canonical
  execution directory or worktree, and a candidate from another directory still
  requires an explicit safe resume decision.

  Background:
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "scope fixture saved"

  @done @folder-discovery-red
  Scenario Outline: Real launches discover candidates in the intended local workspace group
    Given a real scope-discovery fixture for "<layout>"
    And real CLI sessions are saved from the scope-discovery execution directories
    When I launch UDS session discovery from the fixture query directory
    And I send command "list_sessions" with id "scope-discovery-list"
    And I close the UDS connection
    Then the local discovery candidates should contain exactly "<sessions>"

    Examples:
      | layout                     | sessions                         |
      | repository root           | cli:repo-root                    |
      | repository subdirectory   | cli:repo-root,cli:repo-subdir    |
      | nested repository         | cli:nested-repo                  |
      | linked worktree           | cli:repo-root,cli:linked-worktree |
      | non-Git exact folder      | cli:exact-folder                 |
      | symlink canonical identity | cli:canonical,cli:symlink        |
      | detached Git worktree     | cli:repo-root,cli:repo-subdir    |

  @done @folder-discovery-red
  Scenario Outline: Inaccessible Git metadata is an explicit discovery outcome
    Given a real scope-discovery fixture for "<layout>"
    And real CLI sessions are saved from the scope-discovery execution directories
    When I launch UDS session discovery from the fixture query directory
    And I send command "list_sessions" with id "scope-discovery-list"
    And I close the UDS connection
    Then session discovery should report that Git workspace metadata is unavailable

    Examples:
      | layout                     |
      | unavailable Git metadata  |
      | permission-denied Git metadata |
