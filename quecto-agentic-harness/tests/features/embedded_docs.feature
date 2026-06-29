@done
Feature: Embedded capability docs reachable from any directory
  Quecto's own capability docs are embedded in the binary and served by the
  `docs` tool, so an agent can read them regardless of its working directory
  (the previous `read docs/quecto.md` guidance broke whenever quecto ran
  outside its own checkout).

  Scenario: the docs tool lists the embedded capability docs
    When I list the embedded docs
    Then the docs listing should include "quecto"
    And the docs listing should include "subagents"

  Scenario: the docs tool returns an embedded doc by name
    When I read the embedded doc "subagents"
    Then the embedded doc content should contain "agent_cmd"

  Scenario: the docs tool tolerates a .md suffix and docs/ prefix
    When I read the embedded doc "docs/subagents.md"
    Then the embedded doc content should contain "agent_cmd"

  Scenario: an unknown doc name reports the available docs
    When I read the embedded doc "nonexistent"
    Then reading the embedded doc should fail
    And the embedded doc content should contain "quecto"
