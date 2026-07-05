@done @recall @issue-1028
Feature: Recall tool spill retrieval
  As an agent whose large tool outputs have been collapsed out of context
  I want recall to retrieve only the requested session's spilled outputs
  So that context spill recovery is precise, inspectable, and safe across sessions

  Scenario: Recall retrieves the full content for a spilled output id
    Given a recall tool for session "chat-alpha" with spilled output "turn7:bash:0" from tool "bash" preview "cargo test" containing "full cargo test output"
    When I run recall with id "turn7:bash:0"
    Then the recall result should not be an error
    And the recall result should contain "full cargo test output"

  Scenario: Missing spill ids return a tool error instead of empty content
    Given a recall tool for session "chat-alpha" with no spilled outputs
    When I run recall with id "turn404:bash:0"
    Then the recall result should be an error
    And the recall result should contain "No spilled output found for id: turn404:bash:0"

  Scenario: Listing spilled outputs returns an index without full content
    Given a recall tool for session "chat-alpha" with spilled output "turn7:bash:0" from tool "bash" preview "cargo test --all" containing "secret full output"
    When I run recall with id "list"
    Then the recall result should not be an error
    And the recall result should contain "Spilled outputs (1 entries):"
    And the recall result should contain "turn7:bash:0 — cargo test --all"
    And the recall result should not contain "secret full output"

  Scenario: Recall session keys isolate spilled outputs
    Given a recall tool for session "chat-alpha" with spilled output "turn7:bash:0" from tool "bash" preview "alpha command" containing "alpha output"
    And session "chat-beta" has spilled output "turn7:bash:0" from tool "bash" preview "beta command" containing "beta output"
    When I switch the recall tool to session "chat-beta"
    And I run recall with id "turn7:bash:0"
    Then the recall result should not be an error
    And the recall result should contain "beta output"
    And the recall result should not contain "alpha output"

  Scenario: Listing an empty session reports that no spilled outputs exist
    Given a recall tool for session "chat-empty" with no spilled outputs
    When I run recall with id "list"
    Then the recall result should not be an error
    And the recall result should contain "No spilled outputs in this session."
