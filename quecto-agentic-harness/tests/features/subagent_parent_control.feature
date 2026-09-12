@done @subagent-teardown @parent-control
Feature: Launch-bound parent control and the owned-child supervisor (#1935)
  A launcher-created harness is lifetime-scoped to the harness that launched
  it. The launcher mints one random, generation-scoped, non-persisted
  capability per child and delivers it out of band; the child accepts exactly
  one control connection presenting it, and only that connection's loss runs
  the common shutdown. Every directly spawned process is owned by one
  supervisor that attempts the protocol first and signals a retained handle
  at most once per kind; anything without a handle is never signalled.

  # ── Capability minting and the fail-closed allowlist ────────────────────

  Scenario: A minted credential is random, generation-scoped and redacted
    When the launcher mints two parent control credentials
    Then the two capabilities differ and the second generation is newer
    And a credential renders redacted and only expose yields the material

  Scenario Outline: Presentations that do not match the launch fail closed
    Given a harness launched with parent control generation 3
    When a connection presents generation <generation> with the "<capability>" capability
    Then the presentation is refused as "<rejection>"
    And the harness has no bound parent

    Examples:
      | generation | capability | rejection                                                       |
      | 2          | minted     | parent control generation 2 does not match launch generation 3 |
      | 3          | other      | parent control capability mismatch                              |

  Scenario: A top-level harness refuses every presentation
    Given a harness launched without parent control
    When a connection presents generation 1 with the "minted" capability
    Then the presentation is refused as "this harness has no launch-bound parent"
    And the harness has no bound parent

  Scenario: Exactly one presentation binds; replays and second presenters are refused
    Given a harness launched with parent control generation 5
    When a connection presents generation 5 with the "minted" capability
    Then the presentation is accepted and the harness has a bound parent
    When a connection presents generation 5 with the "minted" capability
    Then the presentation is refused as "a parent control connection is already bound"
    When the bound connection closes
    Then the loss is the bound parent's and the binding is spent
    When a connection presents generation 5 with the "minted" capability
    Then the presentation is refused as "the parent control connection was already lost"

  Scenario: Ordinary connection losses never count, not even the last one
    Given a harness launched with parent control generation 1
    When an ordinary connection closes
    Then the loss is an ordinary client's
    When a connection presents generation 1 with the "minted" capability
    And an ordinary connection closes
    Then the loss is an ordinary client's
    And the harness has a bound parent

  Scenario: Capability material must be exactly 64 lowercase hex characters
    Then the capability "<63 a>" is refused for its length
    And the capability "<64 A>" is refused for not being lowercase hex
    And the capability "<64 a>" is accepted

  # ── Out-of-band delivery ──────────────────────────────────────────────────

  Scenario: The sidecar is private, single-use and never survives the child's startup
    When the launcher writes a parent control sidecar
    Then the sidecar has mode 0600 and cannot be written twice
    When the child takes the sidecar
    Then the taken credential equals the minted one and the sidecar is gone
    And taking the sidecar again fails as unreadable

  Scenario Outline: Malformed sidecars are refused and still removed
    Given a sidecar containing '<body>'
    When the child takes the sidecar
    Then taking the sidecar fails with "<error>" and the sidecar is gone

    Examples:
      | body                                              | error                 |
      | not json                                          | malformed             |
      | {"format":2,"generation":1,"capability":"aa"}     | format 2 unsupported  |
      | {"format":1,"generation":1,"capability":"zz"}     | must be 64 hex        |

  Scenario: The presentation frame claims only its own type
    When the launcher mints two parent control credentials
    Then the presentation frame for the first credential round-trips
    And the lines '{"type":"prompt","message":"x"}', '[1]', 'nope' are not presentations
    And the line '{"type":"bind_parent_control","generation":1}' is a malformed presentation
    And the child launch argv carries only the sidecar path, never the material

  # ── The composed harness: bound loss versus ordinary disconnects ─────────

  Scenario: Only the bound parent's EOF ends a launched harness
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And a launch-bound UDS harness launched with a parent control credential
    When an ordinary inspector connects and disconnects
    Then the launched harness is still running
    When the parent connects and presents its credential
    Then the parent receives the bind_parent_control acknowledgement
    When an impostor presents the same credential
    Then the impostor's connection is closed by the harness
    And the launched harness is still running
    When an ordinary inspector connects and disconnects
    Then the launched harness is still running
    When the parent connection closes
    Then the launched harness exits within 10 seconds

  Scenario: A malformed presentation closes that connection and the harness keeps running
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And a launch-bound UDS harness launched with a parent control credential
    When an impostor presents a malformed bind_parent_control frame
    Then the impostor's connection is closed by the harness
    And the launched harness is still running
    When the parent connects and presents its credential
    And the parent connection closes
    Then the launched harness exits within 10 seconds

  Scenario: Bound parent loss during a running turn cancels the turn and exits
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM will delay its response by 20 seconds
    And a launch-bound UDS harness launched with a parent control credential
    When the parent connects and presents its credential
    And an ordinary client sends a prompt that the harness starts working on
    And the parent connection closes
    Then the launched harness exits within 10 seconds

  Scenario: A launched harness whose parent never binds ends at the bind deadline
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And a launch-bound UDS harness launched with a parent control credential and a triggered bind deadline
    When an ordinary inspector connects and disconnects
    Then the launched harness is still running
    When the bind deadline passes
    Then the launched harness exits within 10 seconds
    And a late parent presentation is refused by the exited harness

  Scenario: A shutdown command from the bound parent is acknowledged before the harness exits
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And a launch-bound UDS harness launched with a parent control credential
    When the parent connects and presents its credential
    And the parent sends shutdown "parent_shutdown" with id "p-1"
    Then the parent receives a shutdown acknowledgement for "p-1" with reason "parent_shutdown"
    And the launched harness exits within 10 seconds

  # ── The owned-child supervisor with real processes ───────────────────────

  Scenario: The protocol is always attempted first; TERM then KILL only after a negative outcome
    Given the supervisor adopts a sleeping child
    When the child is terminated with a negative protocol outcome "no socket"
    Then no signal was sent before the protocol outcome
    And the child exited after TERM authorised by "no socket"
    And the supervisor sent exactly "TERM" to the child
    And the supervisor no longer retains the child

  Scenario: KILL follows TERM when the child ignores it
    Given the supervisor adopts a child that ignores TERM
    When the child is terminated with a negative protocol outcome "unreachable"
    Then the child exited after KILL
    And the supervisor sent exactly "TERM, KILL" to the child

  Scenario: An acknowledged child that exits by itself is never signalled
    Given the supervisor adopts a child that exits when told
    When the child is terminated with an acknowledged protocol outcome that tells it
    Then the child exited after the protocol
    And the supervisor sent exactly "" to the child

  Scenario: An acknowledged child that lingers past the exit deadline is terminated
    Given the supervisor adopts a sleeping child
    When the child is terminated with an acknowledged protocol outcome
    Then the child exited after TERM authorised by "acknowledged but not exited"

  Scenario: An already exited child is reaped once and never signalled
    Given the supervisor adopts a child that exits immediately
    And the supervisor has observed the child's exit
    When the child is terminated with a negative protocol outcome "late"
    Then the termination reports the child already exited
    And the supervisor sent exactly "" to the child

  Scenario: Cancelled and concurrent terminations signal at most once per kind
    Given the supervisor adopts a child that ignores TERM
    When a termination is cancelled right after TERM
    And two terminations run concurrently with a negative protocol outcome
    Then the supervisor sent exactly "TERM, KILL" to the child
    And the supervisor no longer retains the child

  Scenario: Nothing without a retained handle is ever signalled
    Given a live fixture process that is not owned by the supervisor
    And a registry entry copied from a reaped owned child
    And a restored registry row carrying the fixture pid
    And a container member row carrying the fixture pid
    When every legacy teardown path runs against those rows
    Then the fixture process is still alive
    And no row requested an owned-child termination
    And terminating an unknown handle reports no retained handle
    And the supervisor sent no signals at all

  Scenario: A detached termination request runs from a plain thread
    Given the supervisor adopts a sleeping child
    When a detached termination is requested from a plain thread
    Then the supervisor reports the child exited by TERM

  # ── Composed parent-death cascade with a real child ──────────────────────

  Scenario: A real child launched by this harness exits on parent loss without any signal
    Given a live SpawnTool and AgentCmdTool backed by a mock LLM child
    When I live-spawn subagent "bound-worker" with initial task "say hello"
    Then the spawn result should not be an error
    And the registry entry for "bound-worker" holds an owned child with a launch generation
    And the parent control sidecar for "bound-worker" was consumed
    And the child argv for "bound-worker" carries --parent-control but no capability material
    When the parent drops its control connection to "bound-worker"
    Then the child process of "bound-worker" exits within 15 seconds
    And the supervisor sent no signals to "bound-worker"
    And the child socket of "bound-worker" was removed by a graceful exit

  Scenario: Termination through the supervisor reaches a real child by protocol first
    Given a live SpawnTool and AgentCmdTool backed by a mock LLM child
    When I live-spawn subagent "protocol-worker" with initial task "say hello"
    Then the spawn result should not be an error
    When the owned child termination of "protocol-worker" is requested
    Then the child process of "protocol-worker" exits within 15 seconds
    And the supervisor sent no signals to "protocol-worker"
    And the child socket of "protocol-worker" was removed by a graceful exit
