Feature: Script-managed direct/proxy liveness and lifecycle parity
  As a parent agent
  I want script-managed children reachable over direct or proxy endpoints and
  their deaths pushed via EOF with exactly one post-mortem inspect
  So that container children appear in status/await exactly like local children without polling

  # Slice 3 of #1369: typed direct/proxy parent endpoint, transactional endpoint
  # ownership, EOF-pushed death, exactly-once inspect updating the authoritative
  # environment aggregate, truthful inspect/cleanup failure persistence.
  # Tag @done is added only when each scenario exercises production behavior.

  @done @container-liveness
  Scenario: Spawning into a proxy-only environment delivers the initial task
    Given proxy-capable script-managed subagent spawning is available
    When I spawn script-managed subagent "proxy-impl-slice3" into a new proxy-only environment with task "PROXY_IMPL_MARKER"
    Then the spawn result should not be an error
    And the spawn result should include environment reference "C1"
    And child "proxy-impl-slice3" should be reachable
    And child "proxy-impl-slice3" should receive "PROXY_IMPL_MARKER"
    And the proxy bridge should have been used at least 1 time

  @done @container-liveness
  Scenario: Proxy-only environment supports prompt and message delivery
    Given proxy-capable script-managed subagent spawning is available
    And script-managed child "proxy-prompt-slice3" is running in a proxy-only environment with task "PROXY_TASK_MARKER"
    When I send prompt "PROXY_PROMPT_MARKER" to child "proxy-prompt-slice3"
    Then the agent command result should not be an error
    And child "proxy-prompt-slice3" should receive "PROXY_PROMPT_MARKER"
    And the proxy bridge should have been used at least 1 time

  @done @container-liveness
  Scenario: Proxy mode never falls back to a decoy direct socket
    Given proxy-capable script-managed subagent spawning is available
    And a decoy direct socket is planted at the requested child socket path
    When I spawn script-managed subagent "proxy-decoy-slice3" into a new proxy-only environment with task "PROXY_DECOY_MARKER"
    Then the spawn result should not be an error
    And child "proxy-decoy-slice3" should receive "PROXY_DECOY_MARKER"
    And the decoy direct socket should have been listening yet received no connections
    And the proxy bridge should have been used at least 1 time

  @done @container-liveness
  Scenario: A create result carrying both direct and proxy endpoints is rejected with rollback
    Given proxy-capable script-managed subagent spawning is available
    And the script-managed create result carries both a socket path and a socket proxy
    When I spawn script-managed subagent "proxy-both-slice3" into a new proxy-only environment with task "PROXY_BOTH_MARKER"
    Then the spawn result should fail because the create result must carry exactly one endpoint
    And the script-managed runtime should have cleaned up an environment exactly 1 time
    And the subagent registry should not contain "proxy-both-slice3"

  @done @container-liveness
  Scenario: Proxy-only child death is pushed via EOF through await with one inspect
    Given proxy-capable liveness script-managed subagent spawning is available
    And script-managed child "proxy-eof-slice3" is running in a proxy-only environment with task "PROXY_EOF_MARKER"
    When the script-managed child "proxy-eof-slice3" is killed behind Quecto's back
    Then awaiting subagent "proxy-eof-slice3" should report status "exited"
    And the script-managed runtime should have inspected an environment exactly 1 time
    And the script-managed runtime should have killed an environment exactly 1 time
    And the container listing should include "C1" with status "stopped" and 0 members

  @done @container-liveness
  Scenario: Child death is pushed via EOF with exactly one inspect and one terminal transition
    Given liveness script-managed subagent spawning is available
    And script-managed child "eof-impl-slice3" is running in an inspectable environment with task "EOF_IMPL_MARKER"
    When the script-managed child "eof-impl-slice3" is killed behind Quecto's back
    Then awaiting subagent "eof-impl-slice3" should report status "exited"
    And the last await reason should be "connection_closed"
    And the script-managed runtime should have inspected an environment exactly 1 time
    And the script-managed runtime should have killed an environment exactly 1 time
    And the container listing should include "C1" with status "stopped" and 0 members

  @done @container-liveness
  Scenario: Child death with no pending await surfaces the note, snapshot, and live event
    Given liveness script-managed subagent spawning is available
    And script-managed child "surface-slice3" is running in an inspectable environment with task "SURFACE_MARKER"
    When the script-managed child "surface-slice3" is killed behind Quecto's back
    Then a passive exit note for "surface-slice3" should be delivered
    And the live event stream should report subagent "surface-slice3" as exited
    And the subagent snapshot should report "surface-slice3" as exited

  @done @container-liveness
  Scenario: Environment registry removal after launch does not break monitoring
    Given liveness script-managed subagent spawning is available
    And script-managed child "race-slice3" is running in an inspectable environment with task "RACE_MARKER"
    And the environment registry entry "C1" has been removed out from under the monitor
    When the script-managed child "race-slice3" is killed behind Quecto's back
    Then awaiting subagent "race-slice3" should report status "exited"

  @done @container-liveness
  Scenario: Inspect result survives zero members and is visible via get_containers
    Given liveness script-managed subagent spawning is available
    And script-managed child "inspectok-slice3" is running in an inspectable environment with task "INSPECT_OK_MARKER"
    When the script-managed child "inspectok-slice3" is killed behind Quecto's back
    Then awaiting subagent "inspectok-slice3" should report status "exited"
    And the container listing entry "C1" should carry inspect metadata "cause" with value "oom-killed"
    And the container listing should include "C1" with status "stopped" and 0 members

  @done @container-liveness
  Scenario: Inspect failure is persisted truthfully with retained context
    Given liveness script-managed subagent spawning is available with an inspect script that fails
    And script-managed child "inspectfail-slice3" is running in an inspectable environment with task "INSPECT_FAIL_MARKER"
    When the script-managed child "inspectfail-slice3" is killed behind Quecto's back
    Then awaiting subagent "inspectfail-slice3" should report status "exited"
    And the script-managed runtime should have inspected an environment exactly 1 time
    And the container listing entry "C1" should record an inspect error

  @done @container-liveness
  Scenario: A member EOF-exit does not kill a shared environment
    Given liveness script-managed subagent spawning is available
    And script-managed child "share-impl-slice3" is running in an inspectable environment with task "SHARE_IMPL_MARKER"
    And read-only subagent "share-obs-slice3" has joined existing environment ref "C1" with task "SHARE_OBS_MARKER"
    When the script-managed child "share-obs-slice3" is killed behind Quecto's back
    Then awaiting subagent "share-obs-slice3" should report status "exited"
    And the script-managed runtime should have inspected an environment exactly 1 time
    And the script-managed runtime should have killed an environment exactly 0 times
    And the container listing should include "C1" with status "running" and 1 member
    And child "share-impl-slice3" should be reachable

  @done @container-liveness
  Scenario: The final member EOF-exit triggers exactly one cleanup claim
    Given liveness script-managed subagent spawning is available
    And script-managed child "final-impl-slice3" is running in an inspectable environment with task "FINAL_IMPL_MARKER"
    And read-only subagent "final-obs-slice3" has joined existing environment ref "C1" with task "FINAL_OBS_MARKER"
    And subagent "final-obs-slice3" has already exited behind Quecto's back
    When the script-managed child "final-impl-slice3" is killed behind Quecto's back
    Then awaiting subagent "final-impl-slice3" should report status "exited"
    And the script-managed runtime should have killed an environment exactly 1 time
    And the script-managed runtime should have inspected an environment exactly 2 times
    And the container listing should include "C1" with status "stopped" and 0 members
