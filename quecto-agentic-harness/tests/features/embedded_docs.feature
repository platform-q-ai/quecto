@done
Feature: Embedded operating manual reachable from any directory
  Quecto's operating manual is embedded in the binary and served by the
  `docs` tool, so an agent can read it regardless of its working directory
  (reading docs from disk breaks whenever quecto runs outside its own checkout).

  Scenario: the docs tool lists the table of contents with titles
    When I list the embedded docs
    Then the docs listing should include "quick-start"
    And the docs listing should include "subagents"
    And the docs listing should include "workflow"
    And the docs listing should include "Quecto parent-agent quick start"

  Scenario: the docs tool returns the quick-start entry page
    When I read the embedded doc "quick-start"
    Then the embedded doc content should contain "Route the work"
    And the embedded doc content should contain "get_messages"

  Scenario: the docs tool returns a concise subagents deep dive
    When I read the embedded doc "subagents"
    Then the embedded doc content should contain "read_only"
    And the embedded doc content should contain "get_messages"

  Scenario: the docs tool tolerates a .md suffix and docs/ prefix
    When I read the embedded doc "docs/quick-start.md"
    Then the embedded doc content should contain "Route the work"

  Scenario: an unknown doc name reports the table of contents
    When I read the embedded doc "nonexistent"
    Then reading the embedded doc should fail
    And the embedded doc content should contain "quick-start"

  Scenario: the subagents deep dive documents the read-only spawn option
    When I read the embedded doc "subagents"
    Then the embedded doc content should contain "read_only"
    And the embedded doc content should contain "not a hard sandbox"
