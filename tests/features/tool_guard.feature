@wip
Feature: Tool guard mechanism
  As an agent operator
  I want tool guards that can intercept and block tool calls before execution
  So that dangerous or premature operations are prevented

  # ─── ToolGuard trait on ToolRegistryImpl ────────────────────────────────────

  Scenario: No guards registered allows all tool execution
    Given a tool registry with core tools
    And no guards are registered
    When I execute the "ls" tool with arguments "{}"
    Then the tool result should not be an error

  Scenario: Guard that allows passes through to tool execution
    Given a tool registry with core tools
    And a guard that allows all calls
    When I execute the "ls" tool with arguments "{}"
    Then the tool result should not be an error

  Scenario: Guard that blocks returns error result with reason
    Given a tool registry with core tools
    And a guard that blocks all calls with reason "blocked by test guard"
    When I execute the "ls" tool with arguments "{}"
    Then the tool result should be an error
    And the tool result should contain "blocked by test guard"

  Scenario: Multiple guards all allow — tool executes
    Given a tool registry with core tools
    And a guard that allows all calls
    And a guard that allows all calls
    When I execute the "ls" tool with arguments "{}"
    Then the tool result should not be an error

  Scenario: Multiple guards first blocks — short-circuits
    Given a tool registry with core tools
    And a guard that blocks all calls with reason "first guard blocked"
    And a guard that allows all calls
    When I execute the "ls" tool with arguments "{}"
    Then the tool result should be an error
    And the tool result should contain "first guard blocked"

  Scenario: Multiple guards second blocks — first passes
    Given a tool registry with core tools
    And a guard that allows all calls
    And a guard that blocks all calls with reason "second guard blocked"
    When I execute the "ls" tool with arguments "{}"
    Then the tool result should be an error
    And the tool result should contain "second guard blocked"

  Scenario: Guard only blocks specific tool
    Given a tool registry with core tools
    And a guard that blocks only "bash" with reason "bash is blocked"
    When I execute the "ls" tool with arguments "{}"
    Then the tool result should not be an error

  Scenario: Guard blocks specific tool when matched
    Given a tool registry with core tools
    And a guard that blocks only "bash" with reason "bash is blocked"
    When I execute the "bash" tool with arguments '{"command": "echo hello"}'
    Then the tool result should be an error
    And the tool result should contain "bash is blocked"

  # ─── WorkflowGuard ─────────────────────────────────────────────────────────

  Scenario: WorkflowGuard allows non-bash tools unconditionally
    Given a workflow guard with commit enforcement at 6
    And no workflow steps are completed
    When the guard checks tool "read" with arguments '{"path": "file.txt"}'
    Then the guard should allow the call

  Scenario: WorkflowGuard allows bash commands that are not git commit or push
    Given a workflow guard with commit enforcement at 6
    And no workflow steps are completed
    When the guard checks tool "bash" with arguments '{"command": "git status"}'
    Then the guard should allow the call

  Scenario: WorkflowGuard blocks git commit when steps incomplete
    Given a workflow guard with commit enforcement at 6
    And no workflow steps are completed
    When the guard checks tool "bash" with arguments '{"command": "git commit -m \"wip\""}'
    Then the guard should block the call
    And the guard block reason should contain "BLOCKED"
    And the guard block reason should contain "workflow"

  Scenario: WorkflowGuard blocks git push when steps incomplete
    Given a workflow guard with commit enforcement at 6
    And no workflow steps are completed
    When the guard checks tool "bash" with arguments '{"command": "git push origin main"}'
    Then the guard should block the call

  Scenario: WorkflowGuard allows git commit when required steps complete
    Given a workflow guard with commit enforcement at 6
    And workflow steps 1 through 6 are completed
    When the guard checks tool "bash" with arguments '{"command": "git commit -m \"feature done\""}'
    Then the guard should allow the call

  Scenario: WorkflowGuard allows git push when required steps complete
    Given a workflow guard with commit enforcement at 6
    And workflow steps 1 through 6 are completed
    When the guard checks tool "bash" with arguments '{"command": "git push origin main"}'
    Then the guard should allow the call

  Scenario: WorkflowGuard allows all commits when enforcement is disabled
    Given a workflow guard with no enforcement
    And no workflow steps are completed
    When the guard checks tool "bash" with arguments '{"command": "git commit -m \"yolo\""}'
    Then the guard should allow the call

  Scenario: WorkflowGuard blocks git commit with config flags
    Given a workflow guard with commit enforcement at 6
    And no workflow steps are completed
    When the guard checks tool "bash" with arguments '{"command": "git -c user.name=test commit -m \"wip\""}'
    Then the guard should block the call

  Scenario: WorkflowGuard blocks chained git commit
    Given a workflow guard with commit enforcement at 6
    And no workflow steps are completed
    When the guard checks tool "bash" with arguments '{"command": "git add . && git commit -m \"wip\""}'
    Then the guard should block the call

  Scenario: WorkflowGuard allows git add without commit
    Given a workflow guard with commit enforcement at 6
    And no workflow steps are completed
    When the guard checks tool "bash" with arguments '{"command": "git add ."}'
    Then the guard should allow the call

  Scenario: WorkflowGuard allows git diff
    Given a workflow guard with commit enforcement at 6
    And no workflow steps are completed
    When the guard checks tool "bash" with arguments '{"command": "git diff"}'
    Then the guard should allow the call

  Scenario: WorkflowGuard allows git log
    Given a workflow guard with commit enforcement at 6
    And no workflow steps are completed
    When the guard checks tool "bash" with arguments '{"command": "git log --oneline"}'
    Then the guard should allow the call

  Scenario: WorkflowGuard block message tells LLM to check status
    Given a workflow guard with commit enforcement at 6
    And no workflow steps are completed
    When the guard checks tool "bash" with arguments '{"command": "git commit -m \"wip\""}'
    Then the guard should block the call
    And the guard block reason should contain "status"


  Scenario: WorkflowGuard not registered when guard_commit is false
    Given a workflow config with guard_commit false and enabled true
    When workflow tools are registered with that config
    Then the tool registry should have 0 guards
