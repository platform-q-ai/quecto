Feature: Shared script-managed environments
  As a parent agent
  I want to add agents to an existing C-ref environment, list environments, and kill them by ref
  So that one session can coordinate shared isolated environments safely

  # Slice 2 of #1369: these scenarios exercise the production join/list/kill
  # stack (spawn tool, environment control use case, script adapters).

  @done @container-env
  Scenario: Read-only observer joins a live environment by ref
    Given shared script-managed subagent spawning is available
    And script-managed child "impl-join-slice2" is running in a shared environment with task "IMPL_JOIN_MARKER"
    When I spawn read-only subagent "observer-join-slice2" into existing environment ref "C1" with task "OBSERVER_JOIN_MARKER"
    Then the spawn result should not be an error
    And the spawn result should include environment reference "C1"
    And the script-managed runtime should have joined an existing environment exactly 1 time
    And the script-managed runtime should have created exactly 1 environment
    And child "observer-join-slice2" should receive "OBSERVER_JOIN_MARKER"
    And subagents "impl-join-slice2" and "observer-join-slice2" should report different agent UUIDs
    And subagents "impl-join-slice2" and "observer-join-slice2" should share environment reference "C1"
    And subagents "impl-join-slice2" and "observer-join-slice2" should share the same workspace
    And subagents "impl-join-slice2" and "observer-join-slice2" should both be listed as members of "C1"

  @done @container-env
  Scenario: Observer joins a live environment by name
    Given shared script-managed subagent spawning is available
    And script-managed child "impl-name-slice2" is running in a shared environment named "review-env" with task "IMPL_NAME_MARKER"
    When I spawn read-only subagent "observer-name-slice2" into existing environment name "review-env" with task "OBSERVER_NAME_MARKER"
    Then the spawn result should not be an error
    And the spawn result should include environment reference "C1"
    And the script-managed runtime should have joined an existing environment exactly 1 time
    And the script-managed runtime should have created exactly 1 environment
    And child "observer-name-slice2" should receive "OBSERVER_NAME_MARKER"

  @done @container-env
  Scenario: Existing join uses the environment's retained script set after the default changes
    Given shared script-managed subagent spawning is available
    And script-managed child "impl-retained-slice2" is running in a shared environment with task "IMPL_RETAINED_MARKER"
    And the configured default container script changes to "alternate"
    When I spawn read-only subagent "observer-retained-slice2" into existing environment ref "C1" with task "OBSERVER_RETAINED_MARKER"
    Then the spawn result should not be an error
    And the join should have used the retained "default" script set

  @done @container-env
  Scenario: Unknown environment ref fails without attempting a join
    Given shared script-managed subagent spawning is available
    And script-managed child "impl-unknown-slice2" is running in a shared environment with task "IMPL_UNKNOWN_MARKER"
    When I spawn read-only subagent "observer-unknown-slice2" into existing environment ref "C9" with task "OBSERVER_UNKNOWN_MARKER"
    Then the spawn result should fail because environment "C9" is unknown
    And the script-managed runtime should have joined an existing environment exactly 0 times
    And the spawn result should not include an environment reference

  @done @container-env
  Scenario: Ambiguous environment name fails without attempting a join
    Given shared script-managed subagent spawning is available
    And script-managed child "impl-dup-a-slice2" is running in a shared environment named "dup-env" with task "IMPL_DUP_A_MARKER"
    And script-managed child "impl-dup-b-slice2" is running in a shared environment named "dup-env" with task "IMPL_DUP_B_MARKER"
    When I spawn read-only subagent "observer-dup-slice2" into existing environment name "dup-env" with task "OBSERVER_DUP_MARKER"
    Then the spawn result should fail because environment name "dup-env" is ambiguous
    And the script-managed runtime should have joined an existing environment exactly 0 times

  @done @container-env
  Scenario: Stopped environment ref fails without attempting a join
    Given shared script-managed subagent spawning is available
    And script-managed child "impl-stopped-slice2" is running in a shared environment with task "IMPL_STOPPED_MARKER"
    And environment "C1" has been killed
    When I spawn read-only subagent "observer-stopped-slice2" into existing environment ref "C1" with task "OBSERVER_STOPPED_MARKER"
    Then the spawn result should fail because environment "C1" is stopped
    And the script-managed runtime should have joined an existing environment exactly 0 times

  @done @container-env
  Scenario: get_containers lists a running environment from the authoritative registry
    Given shared script-managed subagent spawning is available
    And script-managed child "impl-list-slice2" is running in a shared environment with task "IMPL_LIST_MARKER"
    When I run container command "get_containers"
    Then the container command result should not be an error
    And the container listing should include "C1" with status "running" and 1 member

  @done @container-env
  Scenario: get_containers keeps listing a stopped environment
    Given shared script-managed subagent spawning is available
    And script-managed child "impl-liststop-slice2" is running in a shared environment with task "IMPL_LISTSTOP_MARKER"
    And environment "C1" has been killed
    When I run container command "get_containers"
    Then the container command result should not be an error
    And the container listing should include "C1" with status "stopped" and 0 members

  @done @container-env
  Scenario: kill_container terminates all members and calls the retained kill exactly once
    Given shared script-managed subagent spawning is available
    And script-managed child "impl-kill-slice2" is running in a shared environment with task "IMPL_KILL_MARKER"
    And read-only subagent "observer-kill-slice2" has joined existing environment ref "C1" with task "OBSERVER_KILL_MARKER"
    When I kill container "C1"
    Then the container command result should not be an error
    And the script-managed runtime should have killed an environment exactly 1 time
    And child "impl-kill-slice2" should not be reachable
    And child "observer-kill-slice2" should not be reachable
    And the container listing should include "C1" with status "stopped" and 0 members

  @done @container-env
  Scenario: Kill failure persists a retryable cleanup-failed state
    Given shared script-managed subagent spawning is available with a kill script that fails once
    And script-managed child "impl-killfail-slice2" is running in a shared environment with task "IMPL_KILLFAIL_MARKER"
    When I kill container "C1"
    Then the container command result should be an error mentioning "cleanup"
    And the container listing should include "C1" with status "cleanup-failed" and a last error

  @done @container-env
  Scenario: A failed environment cleanup can be retried to completion
    Given shared script-managed subagent spawning is available with a kill script that fails once
    And script-managed child "impl-killretry-slice2" is running in a shared environment with task "IMPL_KILLRETRY_MARKER"
    And the first kill of environment "C1" has failed
    When I kill container "C1"
    Then the container command result should not be an error
    And the container listing should include "C1" with status "stopped" and 0 members
    And the script-managed runtime should have killed an environment exactly 2 times

  @done @container-env
  Scenario: Environment refs are never reused after stop
    Given shared script-managed subagent spawning is available
    And script-managed child "impl-refs-slice2" is running in a shared environment with task "IMPL_REFS_MARKER"
    And environment "C1" has been killed
    When I spawn script-managed subagent "impl-second-slice2" into a new shared environment with task "SECOND_ENV_MARKER"
    Then the spawn result should not be an error
    And the spawn result should include environment reference "C2"

  @done @container-env
  Scenario: Killing a non-final member leaves the environment running
    Given shared script-managed subagent spawning is available
    And script-managed child "impl-final-slice2" is running in a shared environment with task "IMPL_FINAL_MARKER"
    And read-only subagent "observer-final-slice2" has joined existing environment ref "C1" with task "OBSERVER_FINAL_MARKER"
    When I kill subagent "observer-final-slice2"
    Then the script-managed runtime should have killed an environment exactly 0 times
    And the container listing should include "C1" with status "running" and 1 member

  @done @container-env
  Scenario: Killing the final member triggers exactly one environment cleanup
    Given shared script-managed subagent spawning is available
    And script-managed child "impl-final2-slice2" is running in a shared environment with task "IMPL_FINAL2_MARKER"
    And read-only subagent "observer-final2-slice2" has joined existing environment ref "C1" with task "OBSERVER_FINAL2_MARKER"
    And subagent "observer-final2-slice2" has been killed
    When I kill subagent "impl-final2-slice2"
    Then the script-managed runtime should have killed an environment exactly 1 time
    And the container listing should include "C1" with status "stopped" and 0 members
