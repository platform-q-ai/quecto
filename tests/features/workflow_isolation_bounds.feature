@wip
Feature: Workflow isolation bounds (PRD Stage E)
  Recursion is bounded: depth, per-parent concurrency, and a shared token
  budget are enforced so a runaway workflow cannot fork unboundedly or exhaust
  the budget. Exceeding a bound is a structured error, never a silent stop.

  # R-E2 — depth + concurrency caps
  Scenario: spawning beyond max_depth is rejected with a structured error
    Given a subagent spawn whose remaining depth budget is 0
    When the agent attempts to spawn a deeper subagent
    Then the spawn should fail with a structured depth-limit error

  Scenario: a spawn within the depth budget decrements the remaining depth for its child
    Given a subagent spawn whose remaining depth budget is 2
    When the agent spawns a child
    Then the child's remaining depth budget should be 1

  Scenario: exceeding the per-parent concurrency cap is rejected
    Given a parent already at its subagent concurrency cap
    When the agent attempts to spawn another subagent
    Then the spawn should fail with a structured concurrency-limit error

  # R-E1 — budget propagation
  Scenario: budget exhaustion yields a blocked verdict
    Given a unit whose token budget is exhausted
    When the unit is awaited
    Then the await verdict status should be "blocked"

  Scenario: budget is drawn from a shared pool across the tree
    Given a tree token budget of 100000 shared between a parent and its children
    When a child consumes 30000 tokens
    Then the remaining tree budget should be 70000
