@done
Feature: Agent CLI — Headless One-Shot Mode
  As a user or automated system
  I want to run a full agent cycle from the command line with a single message
  So that I can use Quecto non-interactively for scripting, testing, and subagent spawning

  Scenario: One-shot message returns LLM response on stdout
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "The answer is 42"
    When I run quecto agent -m "What is the answer?"
    Then the exit code should be 0
    And stdout should contain "The answer is 42"

  Scenario: Missing message flag shows usage error
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    When I run quecto agent with no flags
    Then the exit code should be 1
    And stderr should contain "agent: -m is required for non-interactive mode"

  Scenario: Missing config shows setup instructions
    Given a temp base directory
    When I run quecto agent -m "hello"
    Then the exit code should be 1
    And stderr should contain "config not found"
    And stderr should contain "quecto onboard"

  Scenario: No configured providers shows clear error
    Given a temp base directory
    And a config file with no API keys
    When I run quecto agent -m "hello"
    Then the exit code should be 1
    And stderr should contain "no LLM providers"

  Scenario: System prompt is prepended to conversation
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "I am a pirate assistant"
    When I run quecto agent --system "You are a pirate" -m "Who are you?"
    Then the exit code should be 0
    And stdout should contain "I am a pirate assistant"

  Scenario: Model override via flag
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "Hello from override model"
    When I run quecto agent --model gpt-5-mini -m "Hi"
    Then the exit code should be 0
    And stdout should contain "Hello from override model"

  Scenario: QUECTO_BASE_DIR environment variable overrides default base directory
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "env override works"
    When I set QUECTO_BASE_DIR to the temp directory
    And I run quecto agent -m "test"
    Then the exit code should be 0
    And stdout should contain "env override works"

  Scenario: Provider error returns non-zero exit code
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns an HTTP 500 error
    When I run quecto agent -m "hello"
    Then the exit code should be 1
    And stderr should contain "Error"

  # --- Issue #191: --no-session flag ---

  Scenario: --no-session flag is accepted and runs ephemerally
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "ephemeral reply"
    When I run quecto agent --no-session -m "hello"
    Then the exit code should be 0
    And stdout should contain "ephemeral reply"

  Scenario: --no-session and -s are mutually exclusive
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    When I run quecto agent --no-session -s mysession -m "hello"
    Then the exit code should be 1
    And stderr should contain "--no-session and -s are mutually exclusive"

  Scenario: --no-session flag is documented in help
    Given a temp base directory
    When I run quecto help
    Then the exit code should be 0
    And stdout should contain "--no-session"

  # --- Issue #0: --no-sandbox flag ---

  Scenario: --no-sandbox flag is accepted and disables workspace restriction
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "sandbox disabled"
    When I run quecto agent --no-sandbox -m "hello"
    Then the exit code should be 0
    And stdout should contain "sandbox disabled"

  Scenario: --no-sandbox flag parses correctly alongside other flags
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "ok"
    When I run quecto agent --no-sandbox --no-session -m "hello"
    Then the exit code should be 0
    And stdout should contain "ok"

  Scenario: --no-sandbox flag is documented in help
    Given a temp base directory
    When I run quecto help
    Then the exit code should be 0
    And stdout should contain "--no-sandbox"

  # --- Issue #0: --network flag ---

  Scenario: --network flag is accepted and enables network passthrough
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "network enabled"
    When I run quecto agent --network -m "hello"
    Then the exit code should be 0
    And stdout should contain "network enabled"

  Scenario: --network flag parses correctly alongside --no-sandbox
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "ok"
    When I run quecto agent --network --no-sandbox -m "hello"
    Then the exit code should be 0
    And stdout should contain "ok"

  Scenario: --network flag is documented in help
    Given a temp base directory
    When I run quecto help
    Then the exit code should be 0
    And stdout should contain "--network"

  # --- Issue #300: --config flag ---

  Scenario: --config flag loads config from custom path
    Given a temp base directory
    And a config file at a custom path with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "custom config works"
    When I run quecto agent --config <custom-config-path> -m "hello"
    Then the exit code should be 0
    And stdout should contain "custom config works"

  Scenario: --config flag with nonexistent path shows error
    Given a temp base directory
    When I run quecto agent --config /tmp/nonexistent-config.json -m "hello"
    Then the exit code should be 1
    And stderr should contain "config not found"

  Scenario: --config flag is documented in help
    Given a temp base directory
    When I run quecto help
    Then the exit code should be 0
    And stdout should contain "--config"

  Scenario: --config with --model override (flag takes priority)
    Given a temp base directory
    And a config file at a custom path with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "model override works"
    When I run quecto agent --config <custom-config-path> --model gpt-5-mini -m "Hi"
    Then the exit code should be 0
    And stdout should contain "model override works"

  # --no-sandbox uses CWD as workspace root
  # Tested at unit level in agent_no_sandbox_tests.rs::test_resolve_agent_workspace_*
  # BDD scenario is pending because in-process CWD mutation is global and unsafe in
  # a parallel test harness; the logic is fully covered by the unit tests.
  # TODO(no-issue): promote to @done once a subprocess-based step runner is available.
  @pending
  Scenario: --no-sandbox uses the process CWD as workspace root
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "cwd-root"
    When I run quecto agent --no-sandbox -m "hello" from a custom working directory
    Then the exit code should be 0
    And the agent workspace should equal the custom working directory
