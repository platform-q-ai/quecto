@container-persistence
Feature: Environments outlive sessions
  As an operator whose agents create container environments
  I want every environment's ref, name, retained script set and status recorded durably per base
  directory, restored (and checked against the runtime) when a harness starts, reachable from a
  new session through get_containers / existing-mode joins / kill_container, and listable,
  killable and garbage-collectable from the CLI
  So that a container created in one session can be joined or removed from the next, and
  orphaned state directories and exited containers never pile up unaddressably

  Background:
    Given persistent script-managed subagent spawning is available as session "session-one"

  @done @issue-2024 @container-env
  Scenario: A committed environment is restored by a rebuilt registry over the same base dir
    Given script-managed child "impl-persist" is running in a shared environment named "persist-env" with task "IMPL_PERSIST_MARKER"
    Then the durable environment registry should record "C1" with status "running" created by "session-one"
    When the harness is restarted as session "session-two"
    And I run container command "get_containers"
    Then the container command result should not be an error
    And the container listing should include "C1" with status "empty" and 0 members
    And the container listing should mark "C1" as restored from session "session-one"
    And the container listing should carry name "persist-env" for "C1"

  @done @issue-2024 @container-env
  Scenario: Refs stay unique across a restart
    Given script-managed child "impl-ref-a" is running in a shared environment with task "IMPL_REF_A_MARKER"
    When the harness is restarted as session "session-two"
    And I spawn script-managed subagent "impl-ref-b" into a new shared environment with task "IMPL_REF_B_MARKER"
    Then the spawn result should not be an error
    And the spawn result should include environment reference "C2"
    And the durable environment registry should record "C1" with status "running" created by "session-one"
    And the durable environment registry should record "C2" with status "running" created by "session-two"

  @done @issue-2024 @container-env
  Scenario: A restored environment whose container is gone is marked stopped, never dropped
    Given script-managed child "impl-gone" is running in a shared environment with task "IMPL_GONE_MARKER"
    And the fake runtime loses the container of "C1" behind the harness's back
    When the harness is restarted as session "session-two"
    And I run container command "get_containers"
    Then the container listing should include "C1" with status "stopped" and a last error
    And the container listing should include "C1" with a last error mentioning "not found at restore"
    And the durable environment registry should record "C1" with status "stopped" created by "session-one"

  @done @issue-2024 @container-env
  Scenario: A new session joins a restored environment and kills it
    Given script-managed child "impl-join-restored" is running in a shared environment with task "IMPL_JOIN_RESTORED_MARKER"
    When the harness is restarted as session "session-two"
    And I spawn read-only subagent "observer-restored" into existing environment ref "C1" with task "OBSERVER_RESTORED_MARKER"
    Then the spawn result should not be an error
    And the spawn result should include environment reference "C1"
    And the persistent runtime should have joined an existing environment exactly 1 time
    When I kill container "C1"
    Then the container command result should not be an error
    And the persistent runtime should have killed an environment exactly 1 time
    And the durable environment registry should record "C1" with status "stopped" created by "session-one"
    And scenario teardown should leave no fixture processes running

  @done @issue-2024 @container-env
  Scenario: A joiner leaving a restored environment never tears it down
    Given script-managed child "impl-keep" is running in a shared environment with task "IMPL_KEEP_MARKER"
    When the harness is restarted as session "session-two"
    And I spawn read-only subagent "observer-keep" into existing environment ref "C1" with task "OBSERVER_KEEP_MARKER"
    And I kill subagent "observer-keep"
    Then the persistent runtime should have killed an environment exactly 0 times
    And the container listing should include "C1" with status "empty" and 0 members
    When I kill container "C1"
    Then the persistent runtime should have killed an environment exactly 1 time
    And scenario teardown should leave no fixture processes running

  @done @issue-2024 @container-env
  Scenario: quecto container ls lists live environments and --all includes stopped ones
    Given script-managed child "impl-ls" is running in a shared environment named "ls-env" with task "IMPL_LS_MARKER"
    And the durable environment registry also records a stopped environment "C7" named "old-env"
    When I run quecto with arguments "container ls"
    Then the exit code should be 0
    And the container table should list "C1" with name "ls-env" status "empty" config "default" and created-by "session-one"
    And the container table should not list "C7"
    When I run quecto with arguments "container ls --all"
    Then the exit code should be 0
    And the container table should list "C7" with name "old-env" status "stopped" config "default" and created-by "elsewhere"

  @done @issue-2024 @container-env
  Scenario: quecto container kill stops a restored environment by ref or name
    Given script-managed child "impl-cli-kill" is running in a shared environment named "cli-kill-env" with task "IMPL_CLI_KILL_MARKER"
    When I run quecto with arguments "container kill cli-kill-env"
    Then the exit code should be 0
    And the output should contain "killed C1"
    # The creating session is still alive here: it sees its member die and
    # runs its own final-member kill after the CLI's (the scripts are
    # idempotent), so at least one kill — never zero.
    And the persistent runtime should have killed an environment at least 1 time
    And the durable environment registry should record "C1" with status "stopped" created by "session-one"
    When I run quecto with arguments "container kill C1"
    Then the exit code should be 1
    And stderr should contain "environment 'C1' is stopped"

  @done @issue-2024 @container-env
  Scenario: quecto container gc --dry-run lists orphans and gc removes only them
    Given script-managed child "impl-gc" is running in a shared environment with task "IMPL_GC_MARKER"
    And an orphaned environment state dir "env-orphan01" with an exited fake container is planted in the state dir
    And an orphaned environment state dir "env-orphan02" without any container is planted in the state dir
    And a fresh environment state dir "env-fresh04" without any container is planted in the state dir
    And an exited fake container "quecto-env-ghost03" with no state dir is left in the fake runtime
    When I run quecto with arguments "container gc --dry-run"
    Then the exit code should be 0
    And the gc report should list "env-orphan01" as removable
    And the gc report should list "env-orphan02" as removable
    And the gc report should list "env-ghost03" as removable
    And the gc report should keep the environment of "C1" as live
    And the gc report should keep "env-fresh04" as a create in flight
    And the state dir should still contain "env-orphan01"
    And the state dir should still contain "env-orphan02"
    And the fake runtime should still know container "quecto-env-ghost03"
    When I run quecto with arguments "container gc"
    Then the exit code should be 0
    And the state dir should no longer contain "env-orphan01"
    And the state dir should no longer contain "env-orphan02"
    And the fake runtime should no longer know container "quecto-env-orphan01"
    And the fake runtime should no longer know container "quecto-env-ghost03"
    And the state dir should still contain the environment of "C1"
    And the state dir should still contain "env-fresh04"
    And the fake runtime should still know the container of "C1"

  @done @issue-2024 @container-env
  Scenario: quecto container gc runs the retained cleanup of a stopped record whose state dir lingers
    Given script-managed child "impl-gc-stopped" is running in a shared environment with task "IMPL_GC_STOPPED_MARKER"
    And the fake runtime loses the container of "C1" behind the harness's back
    When I run quecto with arguments "container gc --dry-run"
    Then the exit code should be 0
    And the gc report should list the environment of "C1" as removable via its retained cleanup
    When I run quecto with arguments "container gc"
    Then the exit code should be 0
    And the persistent runtime should have cleaned up an environment exactly 1 time
    And the state dir should no longer contain the environment of "C1"

  @done @issue-2024 @container-env @serial
  Scenario: A real harness's environment survives its restart and is joined from the next session
    Given a real quecto agent driven by a fake provider spawns container true and is then killed without cleanup
    Then the durable environment registry should record "C1" with status "running" created by "cli:persist-one"
    When a real quecto agent started as session "persist-two" is driven by the fake provider to list containers and join "C1"
    Then the real agent should have exited successfully
    And the tool result the fake provider received should list container "C1" as restored
    And the tool result the fake provider received should be a join into "C1"
    When I run quecto with arguments "container ls"
    Then the exit code should be 0
    And the container table should list "C1" with name "-" status "empty" config "default" and created-by "cli:persist-one"
    When I run quecto with arguments "container kill C1"
    Then the exit code should be 0
    And the persistent runtime should have killed an environment exactly 1 time
    And scenario teardown should leave no fixture processes running
