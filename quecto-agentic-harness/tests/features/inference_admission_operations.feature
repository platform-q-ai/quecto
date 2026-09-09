@inference-admission @done
Feature: Admission authority operations
  Agent processes negotiate bindings, descendants bind pre-registered
  capabilities, operators administer the authority through the CLI, and every
  failure mode is explicit rather than a silent bypass.

  Scenario: An agent process negotiates a root binding and retires it on exit
    Given a running admission authority with capacity two
    When an agent process negotiates a root binding
    And the process reports throttle feedback on an admitted attempt
    Then the authority records the cooldown and one live scope
    When the process shuts down its binding
    Then the authority reports no live scopes and no occupancy

  Scenario: A descendant binds the sidecar its parent wrote and forgeries are refused
    Given a running admission authority with capacity two
    And a parent process with a root binding
    When the parent writes a capability sidecar for a child
    And a child process negotiates with that sidecar
    Then the child is bound and the sidecar is consumed
    And a forged or unreadable sidecar is refused before any attempt

  Scenario: The operator administers the authority through the CLI
    Given a running admission authority with capacity two
    And a configuration file that enables admission for that authority
    When the operator runs the status command
    Then the status JSON reports the epoch and no live scopes
    When the operator runs the reset command
    Then the reset advances the epoch and misuse of the command is refused explicitly

  Scenario: A corrupt, unsupported or missing ledger fails closed on restart
    Given a running admission authority with capacity two
    When the authority stops and its ledger is corrupted
    Then the corrupt ledger is refused as unreadable
    When the ledger is removed after prior operation
    Then a restart is refused until the operator accepts an empty ledger

  Scenario: Roots need the owner token and children cannot self-promote
    Given a running admission authority with capacity one
    And a root session holding an active attempt
    When a connection without the owner token tries to register a root
    Then root registration is refused and the owner token succeeds
    And a retired child capability no longer binds and unknown aliases are refused

  Scenario: A raced cancel completes as a failed attempt instead of leaking the slot
    Given a running admission authority with capacity one
    And a root session holding an active attempt
    When the holder cancels the granted attempt without a transport
    Then the slot is released as a finished attempt with no uncertainty
