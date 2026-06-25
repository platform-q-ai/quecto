@tui @cold-start
Feature: TUI cold-binary first-launch readiness (#808)
  As a quecto-tui user who just ran `cargo install`
  I want the TUI to tolerate a cold kernel binary on first launch
  So that the first start after install does not repeatedly time out

  # The 30s deadline value and the timeout-message composition through
  # `format_agent_startup_failure` are pinned by behavioural unit tests in
  # quecto-tui/src/interface/cli.rs. The run-tui.sh pre-warm and README docs are
  # file-content guarantees verified by tests/repo_docs.rs, not TUI behaviour.

  Scenario: Timeout message names the cold-start cause and warm remedy
    When the agent does not announce its socket before the deadline
    Then the timeout message names the cold-binary first-run cause
    And the timeout message suggests running "quecto --version" to warm the binary
    And the timeout message offers to retry

  Scenario: A starting-agent status is shown while waiting
    When the TUI is waiting for the agent socket path
    Then the TUI surfaces a "starting agent" status indicator
