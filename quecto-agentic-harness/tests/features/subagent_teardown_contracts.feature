@done @subagent-teardown
Feature: Subagent teardown contracts (#1934)
  A launcher-created subagent is lifetime-scoped to the harness that launched
  it. Self shutdown and selected-descendant termination are two different
  operations with typed contracts: shutdown is a two-phase admission whose
  correlated ACK is flushed before anything is torn down, so a parent can
  tell a graceful exit from a loss; selected termination resolves one direct
  edge per hop and never shuts an intermediate down.

  # ── Two-phase admission through the UDS edge ────────────────────────────

  Scenario Outline: Prepare admits, the ACK is flushed, and only then Execute tears down
    Given a harness owning children A and D, where A owns B and C
    When the "<who>" sends shutdown "parent_shutdown" with id "c-1" to the "<how>" harness
    Then the ACK for "c-1" was flushed before any teardown effect
    And the ACK reports reason "parent_shutdown"
    And the teardown cancelled the turn, shut down "A, D", persisted once and signalled exit once
    And the harness lifecycle is "Terminated"

    Examples:
      | who            | how  |
      | bound parent   | idle |
      | local operator | busy |

  Scenario: A failed ACK flush releases the admission and never executes
    Given a harness owning children A and D, where A owns B and C
    And the ACK writer cannot flush
    When the "bound parent" sends shutdown "operator_request" with id "c-2" to the "busy" harness
    Then the shutdown is abandoned and the admission is "released"
    And no teardown effect has run
    And the harness lifecycle is "Accepting"

  Scenario: A failed ACK for a joined command leaves the admission held by the first trigger
    Given a harness owning children A and D, where A owns B and C
    When shutdown is prepared for "termination_signal" by the "signal" trigger
    Given the ACK writer cannot flush
    When the "bound parent" sends shutdown "parent_shutdown" with id "c-3" to the "idle" harness
    Then the shutdown is abandoned and the admission is "still held"
    And no teardown effect has run
    And the harness lifecycle is "Frozen"

  Scenario: A terminated harness refuses a new shutdown with a correlated error
    Given a harness owning children A and D, where A owns B and C
    And the harness has already terminated
    When the "bound parent" sends shutdown "parent_shutdown" with id "c-4" to the "idle" harness
    Then the command is rejected with "harness already terminated" correlated to "c-4"
    And no teardown effect has run

  # ── Two-phase admission at the application boundary ─────────────────────

  Scenario: Dropping the connection task after the ACK still completes the shutdown
    Given a harness owning children A and D, where A owns B and C
    When the connection task is aborted after the ACK is flushed
    Then the teardown cancelled the turn, shut down "A, D", persisted once and signalled exit once
    And the harness lifecycle is "Terminated"

  Scenario: Duplicate Prepare joins the live admission and Execute converges on one outcome
    Given a harness owning children A and D, where A owns B and C
    When shutdown is prepared for "parent_shutdown" by the "protocol" trigger
    And shutdown is prepared for "termination_signal" by the "signal" trigger
    Then both preparations hold one admission and the second joined with its own token
    And the harness lifecycle is "Frozen"
    When the admitted token is executed
    Then the shutdown outcome records reason "parent_shutdown" and triggers "protocol, signal"
    And the teardown cancelled the turn, shut down "A, D", persisted once and signalled exit once
    And a later shutdown preparation joins the completed outcome

  Scenario: Releasing the only holder lifts the freeze and a released token cannot execute
    Given a harness owning children A and D, where A owns B and C
    When shutdown is prepared for "parent_connection_lost" by the "parent-closed" trigger
    And the admitted token is released
    Then the release outcome is "released"
    And the harness lifecycle is "Accepting"
    When the admitted token is released
    Then the release outcome is "not prepared"
    And executing holder 1's token fails with "no shutdown has been prepared"
    When shutdown is prepared for "operator_request" by the "protocol" trigger
    Then executing a token from another harness fails with "shutdown token is not the admitted one"
    And no teardown effect has run

  Scenario: Release is idempotent per holder and the freeze lifts only when every holder released
    Given a harness owning children A and D, where A owns B and C
    When shutdown is prepared for "parent_shutdown" by the "protocol" trigger
    And shutdown is prepared for "termination_signal" by the "signal" trigger
    And holder 1 releases its token
    Then the release outcome is "still held"
    And the harness lifecycle is "Frozen"
    When holder 1 releases its token
    Then the release outcome is "already released"
    And executing holder 1's token fails with "shutdown token was already released"
    When holder 2 releases its token
    Then the release outcome is "released"
    And the harness lifecycle is "Accepting"
    And no teardown effect has run

  Scenario: A detached run dropped by its runtime hands the admission back and the next Execute resumes it
    Given a harness owning children A and D, where A owns B and C
    And the in-flight turn holds cancellation open
    When shutdown is prepared for "parent_shutdown" by the "protocol" trigger
    And the detached run is dropped by its runtime while a joiner waits
    Then the joiner is released with "shutdown execution was interrupted"
    And no child was addressed
    And the harness lifecycle is "Frozen"
    When cancellation is released and the same token is executed again
    Then the executed outcome shut down "A, D"
    And the harness lifecycle is "Terminated"

  Scenario: A spawner with no runtime left reports interruption instead of hanging
    Given a harness owning children A and D, where A owns B and C
    When shutdown is prepared for "operator_request" by the "protocol" trigger
    And the spawner drops the next run unpolled and the token is executed
    Then the joiner is released with "shutdown execution was interrupted"
    And no teardown effect has run
    And the harness lifecycle is "Frozen"
    When the admitted token is executed
    Then the executed outcome shut down "A, D"
    And the harness lifecycle is "Terminated"

  # ── Selected termination routing ────────────────────────────────────────

  Scenario: root → A → B targeting B forwards via A and preserves A and its other children
    Given a harness owning children A and D, where A owns B and C
    When the "bound parent" sends terminate_delegated_agent for "B" generation 1 depth 2 with id "t-b"
    Then the command is forwarded via "A" with remaining depth 1
    And the termination response for "t-b" has status "forwarded"
    And the harness lifecycle is "Accepting"

  Scenario: Targeting a direct child uses self shutdown on that child only
    Given a harness owning children A and D, where A owns B and C
    When the "local operator" sends terminate_delegated_agent for "A" generation 1 depth 1 with id "t-a"
    Then child "A" is asked to shut down for "selected_termination"
    And the termination response for "t-a" has status "shutdown_requested"
    And the harness lifecycle is "Accepting"

  Scenario Outline: Refused routes touch no edge
    Given a harness owning children A and D, where A owns B and C
    When the "bound parent" sends terminate_delegated_agent for "<target>" generation <generation> depth <depth> with id "<id>"
    Then the termination is refused with "<error>" correlated to "<id>"
    And no child was addressed
    And the harness lifecycle is "Accepting"

    Examples:
      | target | generation | depth | id  | error                                          |
      | A      | 9          | 1     | t-1 | stale generation 9 for A (current 1)           |
      | B      | 1          | 1     | t-2 | target B not reachable within 1 remaining hop  |
      | ghost  | 1          | 3     | t-3 | unknown delegated agent ghost                  |
      | root   | 1          | 3     | t-4 | target is the receiving harness; use shutdown  |

  Scenario Outline: Zero and excess depth are refused before any use case runs
    Given a harness owning children A and D, where A owns B and C
    When the "bound parent" sends terminate_delegated_agent for "A" generation 1 depth <depth> with id "<id>"
    Then the command is rejected with "<error>" correlated to "<id>"
    And no child was addressed

    Examples:
      | depth | id  | error                                  |
      | 0     | d-0 | remaining_depth must be at least 1     |
      | 33    | d-1 | remaining_depth 33 exceeds maximum 32  |

  Scenario: A cyclic lineage never yields an edge
    Given a harness whose lineage records X under Y and Y under X
    When the "bound parent" sends terminate_delegated_agent for "X" generation 1 depth 8 with id "t-c"
    Then the termination is refused with "lineage cycle at X" correlated to "t-c"
    And no child was addressed

  Scenario: A frozen harness refuses to route a selected termination
    Given a harness owning children A and D, where A owns B and C
    When shutdown is prepared for "parent_shutdown" by the "protocol" trigger
    And the "bound parent" sends terminate_delegated_agent for "A" generation 1 depth 1 with id "t-f"
    Then the termination is refused with "harness is not accepting control commands" correlated to "t-f"
    And no child was addressed

  Scenario: Child and persistence failures are reported, never fatal to the exit
    Given a harness owning children A and D, where A owns B and C
    And child "A" is unreachable
    And session persistence fails with "disk full"
    When shutdown is prepared for "parent_shutdown" by the "protocol" trigger
    And the admitted token is executed
    Then the outcome reports child "A" failed and persistence failed with "disk full"
    And the executed outcome shut down "D"
    And the harness lifecycle is "Terminated"

  Scenario: A duplicated intermediate makes the lineage ambiguous and no edge is taken
    Given a harness whose lineage lists B under both A and D, and C under B
    When the "bound parent" sends terminate_delegated_agent for "C" generation 1 depth 8 with id "t-amb"
    Then the termination is refused with "ambiguous lineage at B" correlated to "t-amb"
    And no child was addressed

  Scenario: A repository that lost the freeze is a lifecycle violation, after exit readiness
    Given a harness owning children A and D, where A owns B and C
    When shutdown is prepared for "parent_shutdown" by the "protocol" trigger
    Given the lifecycle repository loses the freeze behind the transaction's back
    When the admitted token is executed
    Then the execution fails with "lifecycle violation: cannot terminate from Accepting"

  Scenario: An unreachable direct child target is reported in the capability's own words
    Given a harness owning children A and D, where A owns B and C
    And child "A" is unreachable
    When the "bound parent" sends terminate_delegated_agent for "A" generation 1 depth 1 with id "t-u"
    Then the termination is refused with "direct child A unreachable: unreachable: peer gone" correlated to "t-u"

  Scenario: An unreachable direct child is reported in the capability's own words
    Given a harness owning children A and D, where A owns B and C
    And child "A" is unreachable
    When selected termination of B with depth 2 is requested through the use case
    Then the termination is refused with "direct child A unreachable: unreachable: peer gone" correlated to ""

  # ── Parse → authorize → map at the edge ─────────────────────────────────

  Scenario: A malformed command that still carries an id gets a correlated rejection
    Given a harness owning children A and D, where A owns B and C
    When the bound parent sends the raw line "{\"type\":\"shutdown\",\"id\":\"bad-1\",\"reason\":7}"
    Then the command is rejected with "malformed teardown command" correlated to "bad-1"
    And no teardown effect has run

  Scenario: A non-string id is rejected with the id rendered as text
    Given a harness owning children A and D, where A owns B and C
    When the bound parent sends the raw line "{\"type\":\"shutdown\",\"id\":42,\"reason\":\"parent_shutdown\"}"
    Then the command is rejected with "id must be a string" correlated to "42"
    And no teardown effect has run

  Scenario: An unknown shutdown reason is refused without effect
    Given a harness owning children A and D, where A owns B and C
    When the "bound parent" sends shutdown "kill" with id "r-1" to the "idle" harness
    Then the command is rejected with "unknown shutdown reason" correlated to "r-1"
    And no teardown effect has run
    And the harness lifecycle is "Accepting"

  Scenario: An unauthenticated principal is refused before any use case runs
    Given a harness owning children A and D, where A owns B and C
    When the "unauthenticated client" sends shutdown "parent_shutdown" with id "u-1" to the "idle" harness
    Then the command is rejected with "unauthorized connection" correlated to "u-1"
    And no teardown effect has run
    And the harness lifecycle is "Accepting"

  Scenario: A claimed line over the teardown cap is refused with its correlation id
    Given a harness owning children A and D, where A owns B and C
    When the bound parent sends a shutdown line one byte over the teardown cap
    Then the command is rejected with "exceeds cap 1024" correlated to "big"
    And no teardown effect has run

  Scenario Outline: Lines this edge does not claim are ignored without a frame
    Given a harness owning children A and D, where A owns B and C
    When the bound parent sends the raw line "<line>"
    Then the line is ignored and no frame is written
    And no teardown effect has run

    Examples:
      | line                                         |
      | {\"type\":\"prompt\",\"message\":\"hi\"} |
      | not json                                     |
      | []                                           |
      | {\"reason\":\"parent_shutdown\"}           |

  # ── Pure lifecycle invariants ────────────────────────────────────────────

  Scenario: Accepting freezes, thaws back, and terminates only from Frozen
    Given a harness lifecycle of "Accepting"
    Then the lifecycle accepts new work: "yes"
    When the lifecycle is asked to "freeze"
    Then the lifecycle is "Frozen"
    And the lifecycle accepts new work: "no"
    When the lifecycle is asked to "thaw"
    Then the lifecycle is "Accepting"
    When the lifecycle is asked to "freeze"
    And the lifecycle is asked to "terminate"
    Then the lifecycle is "Terminated"
    And the lifecycle accepts new work: "no"
    And every canonical shutdown reason round-trips and "kill" is unknown

  Scenario Outline: Illegal transitions are refused
    Given a harness lifecycle of "<from>"
    When the lifecycle is asked to "<verb>"
    Then the transition is refused with "<message>"

    Examples:
      | from       | verb      | message                          |
      | Accepting  | terminate | cannot terminate from Accepting  |
      | Terminated | freeze    | cannot freeze from Terminated    |
      | Terminated | thaw      | cannot thaw from Terminated      |
