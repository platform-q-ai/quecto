Feature: Canonical container-runtime scripts and multi-PR orchestration
  As a Quecto operator
  I want to copy the repository's canonical container-runtime script set and
  coordinate separate PR/repository environments from one parent session
  So that documented runtime behavior is proven end to end through the production adapter

  # Slice 5 of #1369: the epic acceptance suite. Every scenario drives the
  # CANONICAL reference scripts at scripts/container-runtime/{create,exec,
  # inspect,kill}.sh through the production script adapter and strict parser.
  # The scripts run in their host-local mode so the suite passes in CI where
  # Docker may be unavailable. Tag @done is added only when each scenario
  # exercises production behavior.

  @done @container-runtime
  Scenario: The canonical create script launches a child through the production adapter
    Given repository fixtures "repo-a" and "repo-b" exist
    And the canonical container-runtime script set is configured
    And the parent session's repository is fixture "repo-a"
    When I spawn canonical subagent "canon-create-slice5" into a new environment with task "CANON_CREATE_MARKER"
    Then the spawn result should not be an error
    And the spawn result should include environment reference "C1"
    And the canonical runtime should have recorded exactly 1 create invocation
    And child "canon-create-slice5" should receive "CANON_CREATE_MARKER"
    And the workspace checkout for "canon-create-slice5" should contain repository marker "REPO_A_MARKER"
    And the child process for "canon-create-slice5" should be running inside its environment checkout

  @done @container-runtime
  Scenario: A failed creation rolls its canonical environment state back
    Given repository fixtures "repo-a" and "repo-b" exist
    And the canonical container-runtime script set is configured
    And the parent session's repository is fixture "repo-a"
    When I spawn canonical subagent "canon-fail-slice5" for a missing repository with task "CANON_FAIL_MARKER"
    Then the spawn result should be a canonical create failure
    And the canonical state root should contain no environment directories

  @done @container-runtime
  Scenario: Two PR environments in one repository get distinct refs and workspaces
    Given repository fixtures "repo-a" and "repo-b" exist
    And the canonical container-runtime script set is configured
    And the parent session's repository is fixture "repo-a"
    And canonical subagent "pr-one-slice5" is running in a new environment with task "PR_ONE_MARKER"
    When I spawn canonical subagent "pr-two-slice5" into a new environment with task "PR_TWO_MARKER"
    Then the spawn result should not be an error
    And the spawn result should include environment reference "C2"
    And the workspaces of "pr-one-slice5" and "pr-two-slice5" should be different
    And the canonical runtime should have recorded exactly 2 create invocations
    And child "pr-one-slice5" should receive "PR_ONE_MARKER"
    And child "pr-two-slice5" should receive "PR_TWO_MARKER"

  @done @container-runtime
  Scenario: Multi-repository creation preserves each requested repository
    Given repository fixtures "repo-a" and "repo-b" exist
    And the canonical container-runtime script set is configured
    And the parent session's repository is fixture "repo-a"
    And canonical subagent "repo-a-slice5" is running for repository fixture "repo-a" with task "MULTI_A_MARKER"
    When I spawn canonical subagent "repo-b-slice5" for repository fixture "repo-b" with task "MULTI_B_MARKER"
    Then the spawn result should not be an error
    And the workspace checkout for "repo-a-slice5" should contain repository marker "REPO_A_MARKER"
    And the workspace checkout for "repo-b-slice5" should contain repository marker "REPO_B_MARKER"
    And the workspaces of "repo-a-slice5" and "repo-b-slice5" should be different

  @done @container-runtime
  Scenario: A read-only reviewer joins the intended environment and sees its shared checkout
    Given repository fixtures "repo-a" and "repo-b" exist
    And the canonical container-runtime script set is configured
    And the parent session's repository is fixture "repo-a"
    And canonical subagent "canon-impl-slice5" is running in a new environment with task "CANON_IMPL_MARKER"
    When I spawn read-only subagent "canon-reviewer-slice5" into existing environment ref "C1" with task "CANON_REVIEWER_MARKER"
    Then the spawn result should not be an error
    And the canonical runtime should have recorded exactly 1 exec invocation
    And child "canon-reviewer-slice5" should receive "CANON_REVIEWER_MARKER"
    And subagents "canon-impl-slice5" and "canon-reviewer-slice5" should report different agent UUIDs
    And subagents "canon-impl-slice5" and "canon-reviewer-slice5" should share environment reference "C1"
    And subagents "canon-impl-slice5" and "canon-reviewer-slice5" should share the same workspace
    And the workspace checkout for "canon-reviewer-slice5" should contain repository marker "REPO_A_MARKER"
    And the canonical exec invocations should target the environment of "canon-impl-slice5"
    And the child process for "canon-reviewer-slice5" should be running inside its environment checkout

  @done @container-runtime
  Scenario: kill_container tears an environment down through the canonical kill script
    Given repository fixtures "repo-a" and "repo-b" exist
    And the canonical container-runtime script set is configured
    And the parent session's repository is fixture "repo-a"
    And canonical subagent "canon-kill-slice5" is running in a new environment with task "CANON_KILL_MARKER"
    When I kill container "C1"
    Then the container command result should not be an error
    And the canonical runtime should have recorded exactly 1 "kill" operation for the environment of "canon-kill-slice5"
    And the canonical runtime should have recorded exactly 0 "cleanup" operations for the environment of "canon-kill-slice5"
    And child "canon-kill-slice5" should not be reachable
    And the container listing should include "C1" with status "stopped" and 0 members

  @done @container-runtime
  Scenario: Environment death is detected without polling and inspected exactly once
    Given repository fixtures "repo-a" and "repo-b" exist
    And the canonical container-runtime script set is configured
    And the parent session's repository is fixture "repo-a"
    And canonical subagent "canon-death-slice5" is running in a new environment with task "CANON_DEATH_MARKER"
    When the canonical child "canon-death-slice5" is killed behind Quecto's back
    Then the subagent snapshot should report "canon-death-slice5" as exited
    And the canonical runtime should have recorded exactly 1 inspect invocation for the environment of "canon-death-slice5"
    And the canonical runtime should have recorded exactly 1 "kill" operation for the environment of "canon-death-slice5"
    And the container listing should include "C1" with status "stopped" and 0 members
    And the TUI renders subagent "canon-death-slice5" as exited

  @done @container-runtime
  Scenario: The container listing shows two PR environments plus a reviewer in one session
    Given repository fixtures "repo-a" and "repo-b" exist
    And the canonical container-runtime script set is configured
    And the parent session's repository is fixture "repo-a"
    And canonical subagent "session-pr1-slice5" is running in a new environment with task "SESSION_PR1_MARKER"
    And canonical subagent "session-pr2-slice5" is running in a new environment with task "SESSION_PR2_MARKER"
    And read-only subagent "session-rev-slice5" has joined existing environment ref "C1" with task "SESSION_REV_MARKER"
    When I run container command "get_containers"
    Then the container command result should not be an error
    And the container listing should include "C1" with status "running" and 2 members
    And the container listing should include "C2" with status "running" and 1 member

  @done @container-runtime
  Scenario: The TUI shows the session's environment layout
    Given repository fixtures "repo-a" and "repo-b" exist
    And the canonical container-runtime script set is configured
    And the parent session's repository is fixture "repo-a"
    And canonical subagent "session-pr1-slice5" is running in a new environment with task "SESSION_PR1_MARKER"
    And canonical subagent "session-pr2-slice5" is running in a new environment with task "SESSION_PR2_MARKER"
    And read-only subagent "session-rev-slice5" has joined existing environment ref "C1" with task "SESSION_REV_MARKER"
    When the TUI renders the session's live subagent state
    Then the TUI panel should group "session-pr1-slice5" and "session-rev-slice5" under environment row "C1"
    And the TUI panel should nest solo member "session-pr2-slice5" beneath environment row "C2"

  @done @container-runtime
  Scenario: Killing one environment leaves the other environment's agents reachable
    Given repository fixtures "repo-a" and "repo-b" exist
    And the canonical container-runtime script set is configured
    And the parent session's repository is fixture "repo-a"
    And canonical subagent "session-pr1-slice5" is running in a new environment with task "SESSION_PR1_MARKER"
    And canonical subagent "session-pr2-slice5" is running in a new environment with task "SESSION_PR2_MARKER"
    And read-only subagent "session-rev-slice5" has joined existing environment ref "C1" with task "SESSION_REV_MARKER"
    When I kill container "C2"
    Then the container command result should not be an error
    And the container listing should include "C2" with status "stopped" and 0 members
    And the container listing should include "C1" with status "running" and 2 members
    And child "session-pr1-slice5" should be reachable
    And child "session-rev-slice5" should be reachable
    And child "session-pr2-slice5" should not be reachable
