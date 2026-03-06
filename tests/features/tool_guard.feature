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

  # ─── Configurable WorkflowGuard ─────────────────────────────────────────────

  Scenario: WorkflowGuard allows non-bash tools unconditionally
    Given a workflow guard with guards:
      | commands    | before_step | message              |
      | git commit  | 7           | Complete steps 1-6.  |
    And no workflow steps are completed
    When the guard checks tool "read" with arguments '{"path": "file.txt"}'
    Then the guard should allow the call

  Scenario: WorkflowGuard allows bash commands not matching any guard rule
    Given a workflow guard with guards:
      | commands    | before_step | message              |
      | git commit  | 7           | Complete steps 1-6.  |
    And no workflow steps are completed
    When the guard checks tool "bash" with arguments '{"command": "git status"}'
    Then the guard should allow the call

  Scenario: WorkflowGuard blocks git commit with custom message
    Given a workflow guard with guards:
      | commands    | before_step | message                               |
      | git commit  | 7           | Finish RED-GREEN-REFACTOR first.      |
    And no workflow steps are completed
    When the guard checks tool "bash" with arguments '{"command": "git commit -m \"wip\""}'
    Then the guard should block the call
    And the guard block reason should contain "Finish RED-GREEN-REFACTOR first."

  Scenario: WorkflowGuard blocks git push when steps incomplete
    Given a workflow guard with guards:
      | commands    | before_step | message                      |
      | git push    | 8           | Push only after committing.  |
    And workflow steps 1 through 6 are completed
    When the guard checks tool "bash" with arguments '{"command": "git push origin main"}'
    Then the guard should block the call
    And the guard block reason should contain "Push only after committing."

  Scenario: WorkflowGuard allows git commit when required steps complete
    Given a workflow guard with guards:
      | commands    | before_step | message              |
      | git commit  | 7           | Complete steps 1-6.  |
    And workflow steps 1 through 7 are completed
    When the guard checks tool "bash" with arguments '{"command": "git commit -m \"feature done\""}'
    Then the guard should allow the call

  Scenario: WorkflowGuard blocks git merge with custom message
    Given a workflow guard with guards:
      | commands    | before_step | message                          |
      | git merge   | 15          | Complete review before merging.  |
    And no workflow steps are completed
    When the guard checks tool "bash" with arguments '{"command": "git merge --squash feature-branch"}'
    Then the guard should block the call
    And the guard block reason should contain "Complete review before merging."

  Scenario: WorkflowGuard blocks gh pr merge
    Given a workflow guard with guards:
      | commands     | before_step | message                 |
      | gh pr merge  | 15          | Finish reviews first.   |
    And no workflow steps are completed
    When the guard checks tool "bash" with arguments '{"command": "gh pr merge 42 --squash --admin"}'
    Then the guard should block the call
    And the guard block reason should contain "Finish reviews first."

  Scenario: WorkflowGuard allows gh pr merge when steps complete
    Given a workflow guard with guards:
      | commands     | before_step | message                 |
      | gh pr merge  | 15          | Finish reviews first.   |
    And workflow steps 1 through 15 are completed
    When the guard checks tool "bash" with arguments '{"command": "gh pr merge 42 --squash"}'
    Then the guard should allow the call

  Scenario: Multiple guard rules — first matches
    Given a workflow guard with guards:
      | commands              | before_step | message                   |
      | git commit, git push  | 7           | Complete steps 1-6.       |
      | git merge             | 15          | Complete review first.    |
    And no workflow steps are completed
    When the guard checks tool "bash" with arguments '{"command": "git commit -m wip"}'
    Then the guard should block the call
    And the guard block reason should contain "Complete steps 1-6."

  Scenario: Multiple guard rules — second matches
    Given a workflow guard with guards:
      | commands              | before_step | message                   |
      | git commit, git push  | 7           | Complete steps 1-6.       |
      | git merge             | 15          | Complete review first.    |
    And workflow steps 1 through 7 are completed
    When the guard checks tool "bash" with arguments '{"command": "git merge --squash feat"}'
    Then the guard should block the call
    And the guard block reason should contain "Complete review first."

  Scenario: Multiple guard rules — both pass when all steps complete
    Given a workflow guard with guards:
      | commands              | before_step | message                   |
      | git commit, git push  | 7           | Complete steps 1-6.       |
      | git merge             | 15          | Complete review first.    |
    And workflow steps 1 through 15 are completed
    When the guard checks tool "bash" with arguments '{"command": "git merge --squash feat"}'
    Then the guard should allow the call

  Scenario: No guard rules means no blocking
    Given a workflow guard with no guard rules
    And no workflow steps are completed
    When the guard checks tool "bash" with arguments '{"command": "git commit -m yolo"}'
    Then the guard should allow the call

  Scenario: WorkflowGuard blocks chained command matching guard rule
    Given a workflow guard with guards:
      | commands    | before_step | message        |
      | git commit  | 7           | Not yet.       |
    And no workflow steps are completed
    When the guard checks tool "bash" with arguments '{"command": "git add . && git commit -m wip"}'
    Then the guard should block the call

  Scenario: WorkflowGuard blocks git commit in multiline command
    Given a workflow guard with guards:
      | commands    | before_step | message        |
      | git commit  | 7           | Not yet.       |
    And no workflow steps are completed
    When the guard checks tool "bash" with arguments '{"command": "echo hello\ngit commit -m wip"}'
    Then the guard should block the call

  Scenario: WorkflowGuard blocks git commit in subshell
    Given a workflow guard with guards:
      | commands    | before_step | message        |
      | git commit  | 7           | Not yet.       |
    And no workflow steps are completed
    When the guard checks tool "bash" with arguments '{"command": "echo $(git commit -m wip)"}'
    Then the guard should block the call

  Scenario: WorkflowGuard blocks git push in backtick subshell
    Given a workflow guard with guards:
      | commands    | before_step | message        |
      | git push    | 8           | Not yet.       |
    And no workflow steps are completed
    When the guard checks tool "bash" with arguments '{"command": "echo `git push origin main`"}'
    Then the guard should block the call

  Scenario: WorkflowGuard allows git add (non-matching subcommand)
    Given a workflow guard with guards:
      | commands    | before_step | message        |
      | git commit  | 7           | Not yet.       |
    And no workflow steps are completed
    When the guard checks tool "bash" with arguments '{"command": "git add ."}'
    Then the guard should allow the call

  Scenario: WorkflowGuard with config flags before subcommand
    Given a workflow guard with guards:
      | commands    | before_step | message        |
      | git commit  | 7           | Not yet.       |
    And no workflow steps are completed
    When the guard checks tool "bash" with arguments '{"command": "git -c user.name=test commit -m wip"}'
    Then the guard should block the call

  Scenario: No guards registered when guards config is empty
    Given a workflow config with empty guards and enabled true
    When workflow tools are registered with that config
    Then the tool registry should have 0 guards
