@done @issue-2024 @inference-admission @serial
Feature: One host-wide admission broker, agent-operable
  One admission broker serves every session on the host. A child inherits its
  parent's authority even when its own config disables admission (#2023), the
  broker installs as a systemd user service, status and reset address the
  global broker whatever the working directory, and an unlisted provider slot
  binds to the default alias.

  Scenario: A child launched with an admission-null config inherits the parent's authority (#2023)
    Given a real admission broker serving a mock provider
    And a real child UDS agent "child" launched with a parent-registered admission context and an admission-null config
    When "child" runs a prompt
    Then "child" completed 1 admitted attempt and reports the authority "connected" in epoch 1
    And the broker reports epoch 1 with at least 2 live scopes

  Scenario: A root survives a reset and a broker restart without restarting; a child fails closed once (#2024 S3)
    Given a real admission broker serving a mock provider
    And a real root UDS agent "root" bound to the broker
    And a real child UDS agent "child" launched with a parent-registered admission context and an admission-null config
    When "root" runs a prompt
    And "child" runs a prompt
    Then "root" completed 1 admitted attempt and reports the authority "connected" in epoch 1
    And "child" completed 1 admitted attempt and reports the authority "connected" in epoch 1
    And the broker reports epoch 1 with at least 3 live scopes
    When the operator resets the broker
    Then "root" reports the authority "reconnecting"
    And "child" reports the authority "unavailable"
    When "root" runs a prompt
    Then "root" completed 2 admitted attempts and reports the authority "connected" in epoch 2
    When "child" runs a prompt
    Then "child"'s prompt failed closed once, telling its parent to respawn it
    And the broker reports epoch 2 with exactly 1 live scope
    When the broker is killed and started again
    Then "root" reports the authority "reconnecting"
    When "root" runs a prompt
    Then "root" completed 3 admitted attempts and reports the authority "connected" in epoch 2
    And the broker reports epoch 2 with exactly 1 live scope

  Scenario: The broker installs and uninstalls as a systemd user service idempotently
    Given a fake systemctl on the PATH
    When the operator installs the broker service with an explicit config
    Then a user unit runs the broker with that absolute config and restarts on failure
    And systemctl was asked to reload, enable and start the unit
    When the operator installs the broker service again
    Then the second install does not rewrite the unchanged unit
    When the operator uninstalls the broker service
    Then the unit is removed and systemctl disabled it
    When the operator uninstalls the broker service again
    Then the uninstall reports nothing to remove

  Scenario: status addresses the global broker whatever the working directory
    Given a running admission broker over a mock provider
    When the operator runs status from an unrelated working directory
    Then the status output names the global authority directory
    When the operator runs status with an explicit --config from an unrelated working directory
    Then the status output names the global authority directory
    When the operator runs status for a directory with no broker
    Then the status output reports that directory as not running

  Scenario: An unlisted provider slot binds to the default alias
    Given an admission config whose only binding is the default alias
    When the broker serves that config
    Then the broker reports the authority as running
