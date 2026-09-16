@issue-2001
Feature: Folder-aware saved-session discovery at the harness boundary
  As a user with sessions created from different folders
  I want the existing saved-session runtime boundary to default to my current folder
  So that opening the resume picker cannot silently offer an unrelated workspace

  @done @folder-local-red
  Scenario: Fieldless session discovery only returns sessions from the configured execution folder
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "saved reply"
    And the real agent runtime saves session "alpha" from configured folder "project-a"
    And the real agent runtime saves session "beta" from configured folder "project-b"
    And the configured execution folder is "project-a"
    When I start the UDS agent with session "current"
    And I send command "list_sessions" with id "local-list"
    And I close the UDS connection
    Then the folder-local list_sessions response should contain exactly "cli:alpha"

  @done @exact-key-compat
  Scenario: An exact opaque session key remains globally resumable
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "saved reply"
    And a saved UDS session "cli:alpha" with 2 messages in the base directory
    And the configured execution folder is "project-b"
    When I start the UDS agent with session "current"
    And I send resume_session "cli:alpha" with id "exact-resume"
    And I close the UDS connection
    Then the resume_session response with id "exact-resume" should carry session key "cli:alpha" and message count 2
