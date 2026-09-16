@issue-2001
Feature: Folder-aware saved-session discovery and exact resume at the harness boundary
  As a user with sessions created from different execution directories
  I want discovery and exact lookup to respect the recorded canonical execution home
  So that same-directory resume is immediate but related or unknown homes cannot mutate my session silently

  @done @folder-local-red
  Scenario: Fieldless session discovery only returns sessions from the UDS launch folder
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "saved reply"
    And the real agent runtime saves session "alpha" from execution folder "project-a"
    And the real agent runtime saves session "beta" from execution folder "project-b"
    And the current execution folder is "project-a"
    When I start the UDS agent with session "current"
    And I send command "list_sessions" with id "local-list"
    And I close the UDS connection
    Then the UDS workspace event should announce execution folder "project-a"
    And the folder-local list_sessions response should contain exactly "cli:alpha"

  @done @exact-key-compat
  Scenario: Exact opaque-key lookup remains globally resolvable
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "saved reply"
    And the real agent runtime saves session "alpha" from execution folder "project-a"
    And the current execution folder is "project-a"
    When I start the UDS agent with session "current"
    And I send resume_session "cli:alpha" with id "compat-resume"
    And I close the UDS connection
    Then the resume_session response with id "compat-resume" should successfully select session key "cli:alpha"

  @done @same-repository-decision-red
  Scenario: A related directory in the same repository requires an explicit foreign disposition
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "saved reply"
    And a Git repository "shared-repo" with execution directories "project-a" and "project-b"
    And the real agent runtime saves session "alpha" from execution folder "shared-repo/project-a"
    And the current execution folder is "shared-repo/project-b"
    When I start the UDS agent with session "current"
    And I send resume_session "cli:alpha" with id "related-resume"
    And I close the UDS connection
    Then the resume_session response with id "related-resume" should require a decision allowing exactly "open_original,fork_current,cancel"

  @done @same-repository-no-mutation-red
  Scenario: A related-directory exact lookup does not mutate the active session while awaiting disposition
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "saved reply"
    And a Git repository "shared-repo" with execution directories "project-a" and "project-b"
    And the real agent runtime saves session "alpha" from execution folder "shared-repo/project-a"
    And the current execution folder is "shared-repo/project-b"
    When I start the UDS agent with session "current"
    And I send get_state with id "before-related"
    And I send command "get_messages" with id "messages-before-related"
    And I send resume_session "cli:alpha" with id "related-plan"
    And I send get_state with id "after-related"
    And I send command "get_messages" with id "messages-after-related"
    And I close the UDS connection
    Then the session keys of the responses with ids "before-related" and "after-related" should match
    And the message histories of responses "messages-before-related" and "messages-after-related" should match

  @done @legacy-decision-red
  Scenario: Exact lookup of a legacy unscoped key returns an explicit association decision
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And a saved UDS session "cli:legacy" with 2 messages in the base directory
    And the current execution folder is "project-b"
    When I start the UDS agent with session "current"
    And I send resume_session "cli:legacy" with id "legacy-resume"
    And I close the UDS connection
    Then the resume_session response with id "legacy-resume" should require a decision allowing at least "associate_current,fork_current,cancel"

  @done @legacy-no-mutation-red
  Scenario: Legacy exact lookup does not mutate the active session while awaiting association
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And a saved UDS session "cli:legacy" with 2 messages in the base directory
    And the current execution folder is "project-b"
    When I start the UDS agent with session "current"
    And I send get_state with id "before-legacy"
    And I send command "get_messages" with id "messages-before-legacy"
    And I send resume_session "cli:legacy" with id "legacy-plan"
    And I send get_state with id "after-legacy"
    And I send command "get_messages" with id "messages-after-legacy"
    And I close the UDS connection
    Then the session keys of the responses with ids "before-legacy" and "after-legacy" should match
    And the message histories of responses "messages-before-legacy" and "messages-after-legacy" should match
